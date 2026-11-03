//! A disjoint entity's writes: insert, update and change of kind, each one
//! statement whose parts are data-modifying `WITH` clauses.
//!
//! Every part writing a table is named as that table ([`DisjointTable::written`]), so
//! the statement's `SELECT` is the read's own projection and joins
//! ([`DisjointShape::project`]) reading each written table through the rows
//! its part wrote, and decodes through the read's decode. Every part writing
//! a kind's table reads the base row's key from the base's part, and that is
//! the only order PostgreSQL holds the parts to: they all see one snapshot,
//! and a part reading another's `RETURNING` runs after it. A base part that
//! touches no row therefore leaves every other part touching none, and the
//! statement returning no row.

use core::marker::PhantomData;
use std::fmt;

use pgorm_query::{
    BinOper, CommonTableExpression, Condition, Expr, FromItem, InsertStatement,
    IntoSubQueryStatement, LockType, Name, Query, ReturningClause, SelectStatement,
    UpdateStatement, Value, WithClause,
};

use super::disjoint::{KindRows, key_filter};
use super::helper::join_condition;
use crate::{
    ActiveValue, Disjoint, DisjointKind, DisjointShape, DisjointTable, DisjointValues, Error,
    QueryTrait,
};

/// A write to a disjoint entity `D`, built by [`Disjoint::insert`],
/// [`Disjoint::update`] or [`Disjoint::change_kind`]: one statement, its
/// parts joined by data-modifying `WITH` clauses, so it applies whole or not
/// at all with no transaction, and the constraints `Schema::create_disjoint`
/// writes are checked once, at its end.
///
/// Its terminal, [`exec_returning_model`](Self::exec_returning_model),
/// returns the pair a read of the written row gives, `(base::Model, D)`: the
/// statement's `SELECT` is [`SelectDisjoint`](crate::SelectDisjoint)'s
/// projection and joins, reading each table the statement wrote through the
/// rows it wrote there.
///
/// Building one sends nothing. Its construction has already run
/// [`check_disjoint`](crate::check_disjoint) and refused what the write must
/// not be given, so a value of this type is a statement that can be sent;
/// [`QueryTrait::build`] renders it.
// [spec:pgorm:sem:disjoint.write+1]
pub struct WriteDisjoint<D: Disjoint> {
    pub(crate) query: SelectStatement,
    marker: PhantomData<fn() -> D>,
}

impl<D: Disjoint> fmt::Debug for WriteDisjoint<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteDisjoint")
            .field("query", &self.query)
            .finish()
    }
}

impl<D: Disjoint> Clone for WriteDisjoint<D> {
    fn clone(&self) -> Self {
        Self {
            query: self.query.clone(),
            marker: PhantomData,
        }
    }
}

impl<D: Disjoint> WriteDisjoint<D> {
    /// The write `query` is, which a shape of `D` built.
    pub(crate) fn of(query: SelectStatement) -> Self {
        Self {
            query,
            marker: PhantomData,
        }
    }
}

/// Which values a write writes: an insert writes `Set` and `Unchanged`
/// values alike, an update `Set` ones alone.
// [spec:pgorm:sem:disjoint.write+1]
#[derive(Clone, Copy, Debug)]
enum Writes {
    /// An insert: everything but `NotSet`.
    Insert,
    /// An update: `Set` alone.
    Update,
}

impl Writes {
    fn writes(self, value: &ActiveValue<Value>) -> bool {
        match self {
            Self::Insert => !value.is_not_set(),
            Self::Update => value.is_set(),
        }
    }
}

/// The value `value` holds, unless it is `NotSet`.
fn held(value: &ActiveValue<Value>) -> Option<Value> {
    match value {
        ActiveValue::Set(value) | ActiveValue::Unchanged(value) => Some(value.clone()),
        ActiveValue::NotSet => None,
    }
}

/// Refuse, as [`Error::Type`] naming the table and column, a value in the
/// column at `index` of `table` that `writes` would write, where the column's
/// value comes from elsewhere for the reason `why` gives.
// [spec:pgorm:sem:disjoint.write+1]
fn unwritten(
    table: &DisjointTable,
    values: &[ActiveValue<Value>],
    index: usize,
    writes: Writes,
    why: &str,
) -> Result<(), Error> {
    if !writes.writes(&values[index]) {
        return Ok(());
    }
    Err(Error::Type(format!(
        "`{}`.`{}` {}: {why}",
        table.label,
        table.columns[index].name.to_string(),
        match writes {
            Writes::Insert => "holds a value",
            Writes::Update => "is `Set`",
        },
    )))
}

/// Refuse a value of a kind row's key that `writes` would write: the row's
/// key is its base row's.
fn unwritten_key(
    table: &DisjointTable,
    values: &[ActiveValue<Value>],
    kind: &DisjointKind,
    writes: Writes,
) -> Result<(), Error> {
    for index in &table.key {
        unwritten(
            table,
            values,
            *index,
            writes,
            &format!(
                "it is the key of the kind `{}`, whose row takes its key from its base row, so \
                 its `ActiveModel` leaves it `NotSet`",
                kind.value
            ),
        )?;
    }
    Ok(())
}

/// The values `kind`'s row is given, refused as [`Error::Type`] when a kind
/// with a table is given none, or one without a table some.
/// A kind's table beside the values its row is given.
type KindRow<'k, 'v> = (&'k DisjointTable, &'v [ActiveValue<Value>]);

fn kind_row<'k, 'v>(
    kind: &'k DisjointKind,
    row: Option<&'v DisjointValues>,
) -> Result<Option<KindRow<'k, 'v>>, Error> {
    match (&kind.table, row) {
        (Some(table), Some(row)) => Ok(Some((table, table.owns(row)?))),
        (None, None) => Ok(None),
        (Some(table), None) => Err(Error::Type(format!(
            "the kind `{}` has a table, `{}`, so a write of it gives its row's values",
            kind.value, table.label
        ))),
        (None, Some(_)) => Err(Error::Type(format!(
            "the kind `{}` has no table, so a write of it gives no row's values",
            kind.value
        ))),
    }
}

/// Every column of `table`, in order, qualified by its bare name: what a part
/// returns, as stored, for the statement's `SELECT` to cast as the read casts
/// it.
///
/// Qualified because a part that also reads another part (`UPDATE .. FROM`,
/// `DELETE .. USING`) sees two relations whose columns may share names.
fn qualified_columns(table: &DisjointTable) -> Vec<(Name, Name)> {
    table
        .columns
        .iter()
        .map(|column| (Name::clone(table.name()), Name::clone(&column.name)))
        .collect()
}

/// The `RETURNING` of a part writing `table`.
// [spec:pgorm:sem:disjoint.write+1]
fn returned(table: &DisjointTable) -> ReturningClause {
    Query::returning().columns(qualified_columns(table))
}

/// The part of a write that writes `table`, named as the table.
// [spec:pgorm:sem:disjoint.write+1]
fn written_part(
    table: &DisjointTable,
    statement: impl IntoSubQueryStatement,
) -> CommonTableExpression {
    CommonTableExpression::new(Name::clone(table.name()), statement)
}

impl DisjointShape {
    /// The condition pairing a row of `table` with the base's part on the
    /// key, column by column.
    fn key_condition(&self, table: &DisjointTable) -> Result<Condition, Error> {
        self.key_relation(table, FromItem::from(table.table.clone()))
            .map(join_condition)
            .ok_or_else(|| {
                Error::Type(format!(
                    "`{}` has no primary key column to key its kinds' rows by",
                    self.base.label
                ))
            })
    }

    /// The statement whose `WITH` holds the base's part and then `kinds`, and
    /// whose `SELECT` reads the base row through the base's part and each
    /// kind's table from where `rows` says.
    fn write(
        &self,
        base: CommonTableExpression,
        kinds: impl IntoIterator<Item = CommonTableExpression>,
        rows: &dyn Fn(usize) -> KindRows,
    ) -> SelectStatement {
        let mut with = WithClause::new(base);
        for part in kinds {
            with.cte(part);
        }
        let mut query = SelectStatement::new();
        query.from(self.base.written());
        self.project(&mut query, rows);
        query.with(with);
        query
    }

    /// The part inserting `kind`'s row from `row`, its key read from the
    /// base's part, or none for a kind without a table.
    // [spec:pgorm:sem:disjoint.write+1]
    fn insert_kind(
        &self,
        kind: &DisjointKind,
        row: Option<&DisjointValues>,
    ) -> Result<Option<CommonTableExpression>, Error> {
        let Some((table, values)) = kind_row(kind, row)? else {
            return Ok(None);
        };
        unwritten_key(table, values, kind, Writes::Insert)?;
        let base = self.base.written();
        let mut columns = Vec::new();
        let mut select = SelectStatement::new();
        for (base_key, kind_key, _) in self.key_pairs(table) {
            columns.push(Name::clone(&kind_key.name));
            select.expr(Expr::col((
                Name::clone(base.table()),
                Name::clone(&base_key.name),
            )));
        }
        for (column, value) in table.columns.iter().zip(values) {
            if let Some(value) = held(value) {
                columns.push(Name::clone(&column.name));
                select.expr(column.casts.save(value));
            }
        }
        select.from(base);
        let mut insert = InsertStatement::new();
        insert
            .into_table(table.table.clone())
            .columns(columns)
            .select_from(select)?
            .returning(returned(table));
        Ok(Some(written_part(table, insert)))
    }

    /// The part updating the set columns of `kind`'s row from `row`, found by
    /// the base part's key, or none for a kind without a table or a row
    /// setting nothing.
    // [spec:pgorm:sem:disjoint.write+1]
    fn update_kind(
        &self,
        kind: &DisjointKind,
        row: Option<&DisjointValues>,
    ) -> Result<Option<CommonTableExpression>, Error> {
        let Some((table, values)) = kind_row(kind, row)? else {
            return Ok(None);
        };
        unwritten_key(table, values, kind, Writes::Update)?;
        let mut update = UpdateStatement::new();
        update.table(table.table.clone());
        for (column, value) in table.columns.iter().zip(values) {
            if let ActiveValue::Set(value) = value {
                update.value(Name::clone(&column.name), column.casts.save(value.clone()));
            }
        }
        if update.get_values().is_empty() {
            return Ok(None);
        }
        update
            .from(self.base.written())
            .cond_where(self.key_condition(table)?)
            .returning(returned(table));
        Ok(Some(written_part(table, update)))
    }

    /// The part deleting the base row's row from `table`.
    // [spec:pgorm:sem:disjoint.write+1]
    fn delete_kind(&self, table: &DisjointTable) -> Result<CommonTableExpression, Error> {
        let mut delete = Query::delete();
        delete
            .from_table(table.table.clone())
            .using(self.base.written())
            .cond_where(self.key_condition(table)?)
            .returning(returned(table));
        Ok(written_part(table, delete))
    }

    /// Insert a value of the kind `kind`: the base row from `base`, its kind
    /// column holding `kind`, and, for a kind with a table, the kind's row
    /// from `row`, its key read from the inserted base row.
    ///
    /// The statement [`Disjoint::insert`] builds, for a shape described at run
    /// time, refusing what it refuses as [`Error::Type`]: a value held in the
    /// base's kind column or in the kind row's key, `row` given for a kind
    /// without a table or not given for one with a table, values of another
    /// table, and a kind the shape lacks. [`check`](Self::check) is the
    /// caller's to run.
    // [spec:pgorm:sem:disjoint.write+1]
    // [spec:pgorm:req:bindings.disjoint+1]
    pub fn insert(
        &self,
        base: &DisjointValues,
        kind: &str,
        row: Option<&DisjointValues>,
    ) -> Result<SelectStatement, Error> {
        let mut values = self.base.owns(base)?.to_vec();
        unwritten(
            &self.base,
            &values,
            self.kind,
            Writes::Insert,
            "it is the kind column, which an insert sets to the kind it is given, so the \
             base's `ActiveModel` leaves it `NotSet`",
        )?;
        let (written, entry) = self.kind(kind)?;
        values[self.kind] = ActiveValue::Set(Value::from(entry.value.clone()));
        let mut insert = Query::insert();
        insert
            .into_table(self.base.table.clone())
            .or_default_values();
        let (columns, saved): (Vec<_>, Vec<_>) = self
            .base
            .columns
            .iter()
            .zip(&values)
            .filter_map(|(column, value)| {
                held(value).map(|value| (Name::clone(&column.name), column.casts.save(value)))
            })
            .unzip();
        if !columns.is_empty() {
            insert.columns(columns).values(saved)?;
        }
        insert.returning(returned(&self.base));
        let row = self.insert_kind(entry, row)?;
        Ok(self.write(written_part(&self.base, insert), row, &|kind| {
            if kind == written {
                KindRows::Written
            } else {
                KindRows::Table
            }
        }))
    }

    /// Update a value of the kind `kind`: the set columns of the base row
    /// from `base` and of its kind's row from `row`, both found by the base's
    /// key, which `base` holds `Set` or `Unchanged`, and narrowed to a row of
    /// the kind `kind`.
    ///
    /// The statement [`Disjoint::update`] builds, for a shape described at run
    /// time, refusing what it refuses: the base's kind column `Set`, and a
    /// kind row's key `Set`, as [`Error::Type`], and a `NotSet` key part of
    /// `base` as [`Error::PrimaryKeyNotSet`], beside what
    /// [`insert`](Self::insert) refuses of `row` and of `kind`. With nothing
    /// set in either, the write is a read of the row of that key and kind.
    // [spec:pgorm:sem:disjoint.write+1]
    // [spec:pgorm:req:bindings.disjoint+1]
    pub fn update(
        &self,
        base: &DisjointValues,
        kind: &str,
        row: Option<&DisjointValues>,
    ) -> Result<SelectStatement, Error> {
        let values = self.base.owns(base)?;
        unwritten(
            &self.base,
            values,
            self.kind,
            Writes::Update,
            "it is the kind column, and an update keeps the row's kind, which `change_kind` \
             changes",
        )?;
        let mut key = Vec::with_capacity(self.base.key.len());
        for index in &self.base.key {
            key.push(held(&values[*index]).ok_or(Error::PrimaryKeyNotSet)?);
        }
        let (current, entry) = self.kind(kind)?;
        let mut update = UpdateStatement::new();
        update.table(self.base.table.clone());
        for (index, (column, value)) in self.base.columns.iter().zip(values).enumerate() {
            if let ActiveValue::Set(value) = value
                && !self.base.key.contains(&index)
            {
                update.value(Name::clone(&column.name), column.casts.save(value.clone()));
            }
        }
        let condition = key_filter(&self.base, key).add(self.kind_eq(entry.value.clone()));
        let row = self.update_kind(entry, row)?;
        let written = row.is_some();
        Ok(
            self.write(self.updated_base(update, condition), row, &|kind| {
                if written && kind == current {
                    KindRows::Written
                } else {
                    KindRows::Table
                }
            }),
        )
    }

    /// The base's part of an update: the `UPDATE` of its set columns,
    /// narrowed by `condition`, or, when it sets none, the `SELECT` of the row
    /// so narrowed `FOR KEY SHARE`.
    ///
    /// Either way the base row is locked before any kind part reads it: the
    /// `UPDATE` by its row lock, the `SELECT` by `FOR KEY SHARE`, which a
    /// change of the row's kind (it writes the kind column, in the base's
    /// `UNIQUE` key) and a delete both wait on. Under `READ COMMITTED` a part
    /// blocked behind one re-checks its `WHERE` against the row that
    /// committed, so a row whose kind changed meanwhile is no row,
    /// `Error::RecordNotFound`, rather than a base row of the old kind
    /// returned beside a kind row the change deleted.
    // [spec:pgorm:sem:disjoint.write+1]
    fn updated_base(
        &self,
        mut update: UpdateStatement,
        condition: Condition,
    ) -> CommonTableExpression {
        if !update.get_values().is_empty() {
            update.cond_where(condition).returning(returned(&self.base));
            return written_part(&self.base, update);
        }
        let mut guard = SelectStatement::new();
        guard
            .columns(qualified_columns(&self.base))
            .from(self.base.table.clone())
            .cond_where(condition)
            .lock(LockType::KeyShare);
        written_part(&self.base, guard)
    }

    /// Change the kind of the row whose key `key` gives, in key order, to
    /// `kind`: delete its row from every other kind's table, its current
    /// kind's among them, insert the new kind's row from `row` where that
    /// kind has a table, and set the base row's kind column, narrowed to a
    /// row of another kind.
    ///
    /// The statement [`Disjoint::change_kind`] builds, for a shape described
    /// at run time, refusing what it refuses as [`Error::Type`], beside a key
    /// of another arity than the base's and what [`insert`](Self::insert)
    /// refuses of `row` and of `kind`.
    // [spec:pgorm:sem:disjoint.write+1]
    // [spec:pgorm:req:bindings.disjoint+1]
    pub fn change_kind(
        &self,
        key: Vec<Value>,
        kind: &str,
        row: Option<&DisjointValues>,
    ) -> Result<SelectStatement, Error> {
        self.key_arity(key.len())?;
        let (new, entry) = self.kind(kind)?;
        let mut parts = Vec::new();
        for (index, kind) in self.kinds.iter().enumerate() {
            if index == new {
                parts.extend(self.insert_kind(kind, row)?);
            } else if let Some(table) = &kind.table {
                parts.push(self.delete_kind(table)?);
            }
        }
        let column = self.kind_column();
        let mut update = UpdateStatement::new();
        update
            .table(self.base.table.clone())
            .value(
                Name::clone(&column.name),
                column.casts.save(entry.value.clone().into()),
            )
            .cond_where(
                key_filter(&self.base, key)
                    .add(self.kind_compare(BinOper::NotEqual, entry.value.clone())),
            )
            .returning(returned(&self.base));
        Ok(
            self.write(written_part(&self.base, update), parts, &|kind| {
                if kind == new {
                    KindRows::Written
                } else {
                    KindRows::Deleted
                }
            }),
        )
    }
}

// [spec:pgorm:sem:disjoint.write+1]
impl<D: Disjoint> QueryTrait for WriteDisjoint<D> {
    type QueryStatement = SelectStatement;

    fn query(&mut self) -> &mut SelectStatement {
        &mut self.query
    }

    fn as_query(&self) -> &SelectStatement {
        &self.query
    }

    fn into_query(self) -> SelectStatement {
        self.query
    }
}

// [spec:pgorm:sem:disjoint.write+1/test]    the SQL of each write for the vehicle fixture: an insert of a payload kind and of a unit kind, an update of both rows, of the base alone, of the kind row alone and of neither, and a change of kind to a payload kind and to the unit kind, each one WITH statement whose SELECT is the read's projection over the parts named as their tables; a composite key pairs column by column
#[cfg(test)]
mod tests {
    use crate::tests_cfg::{
        car, flat,
        listing::{self, Listing, ListingActiveModel},
        truck,
        vehicle::{self, Vehicle, VehicleActiveModel},
    };
    use crate::{ActiveValue, Disjoint, QueryTrait, set};
    use pgorm_query::{Value, Values};
    use pretty_assertions::assert_eq;

    /// The read's projection of a vehicle, which every write's `SELECT` is.
    const PROJECTION: &str = concat!(
        r#"SELECT "vehicle"."id" AS "s0_id", CAST("vehicle"."kind" AS text) AS "s0_kind", "vehicle"."name" AS "s0_name", "#,
        r#""car"."id" AS "s1_id", "car"."seats" AS "s1_seats", "#,
        r#""truck"."vehicle_id" AS "s2_vehicle_id", "truck"."axles" AS "s2_axles", "truck"."payload_kg" AS "s2_payload_kg" "#,
        r#"FROM "vehicle""#,
    );

    const CAR_JOIN: &str = r#"LEFT JOIN "car" ON "vehicle"."id" = "car"."id""#;
    const TRUCK_JOIN: &str = r#"LEFT JOIN "truck" ON "vehicle"."id" = "truck"."vehicle_id""#;
    const VEHICLE_RETURNING: &str =
        r#"RETURNING "vehicle"."id", "vehicle"."kind", "vehicle"."name")"#;
    const CAR_RETURNING: &str = r#"RETURNING "car"."id", "car"."seats")"#;
    const TRUCK_RETURNING: &str =
        r#"RETURNING "truck"."vehicle_id", "truck"."axles", "truck"."payload_kg")"#;

    #[track_caller]
    fn assert_sql(write: &impl QueryTrait, parts: &[&str], values: Vec<Value>) {
        let (sql, bound) = write.build();
        assert_eq!(sql, parts.join(" "));
        assert_eq!(bound, Values(values));
    }

    fn named(name: &str) -> vehicle::ActiveModel {
        vehicle::ActiveModel {
            name: set(name),
            ..Default::default()
        }
    }

    fn keyed(id: i32) -> vehicle::ActiveModel {
        vehicle::ActiveModel {
            id: ActiveValue::Unchanged(id),
            ..Default::default()
        }
    }

    fn seats(seats: i16) -> VehicleActiveModel {
        VehicleActiveModel::Car(car::ActiveModel {
            seats: set(seats),
            ..Default::default()
        })
    }

    fn truck(axles: i16, payload_kg: i32) -> VehicleActiveModel {
        VehicleActiveModel::Truck(truck::ActiveModel {
            axles: set(axles),
            payload_kg: set(payload_kg),
            ..Default::default()
        })
    }

    #[test]
    fn an_insert_keys_its_kind_row_by_base() {
        let write = Vehicle::insert(named("mini"), seats(4)).expect("the insert is built");
        assert_sql(
            &write,
            &[
                r#"WITH "vehicle" AS (INSERT INTO "vehicle" ("kind", "name")"#,
                r#"VALUES (CAST($1::text AS vehicle_kind), $2)"#,
                VEHICLE_RETURNING,
                r#", "car" AS (INSERT INTO "car" ("id", "seats")"#,
                r#"SELECT "vehicle"."id", $3 FROM "vehicle""#,
                CAR_RETURNING,
                PROJECTION,
                CAR_JOIN,
                TRUCK_JOIN,
            ],
            vec!["car".into(), "mini".into(), Value::SmallInt(Some(4))],
        );
    }

    #[test]
    fn a_unit_insert_writes_the_base_alone() {
        let write =
            Vehicle::insert(named("bmx"), VehicleActiveModel::Bike).expect("the insert is built");
        assert_sql(
            &write,
            &[
                r#"WITH "vehicle" AS (INSERT INTO "vehicle" ("kind", "name")"#,
                r#"VALUES (CAST($1::text AS vehicle_kind), $2)"#,
                VEHICLE_RETURNING,
                PROJECTION,
                CAR_JOIN,
                TRUCK_JOIN,
            ],
            vec!["bike".into(), "bmx".into()],
        );
    }

    #[test]
    fn an_update_guards_the_base_by_its_kind() {
        let base = vehicle::ActiveModel {
            name: set("cooper"),
            ..keyed(7)
        };
        let write = Vehicle::update(base, seats(5)).expect("the update is built");
        assert_sql(
            &write,
            &[
                r#"WITH "vehicle" AS (UPDATE "vehicle" SET "name" = $1"#,
                r#"WHERE "vehicle"."id" = $2 AND "vehicle"."kind" = CAST($3::text AS vehicle_kind)"#,
                VEHICLE_RETURNING,
                r#", "car" AS (UPDATE "car" SET "seats" = $4 FROM "vehicle""#,
                r#"WHERE "vehicle"."id" = "car"."id""#,
                CAR_RETURNING,
                PROJECTION,
                CAR_JOIN,
                TRUCK_JOIN,
            ],
            vec![
                "cooper".into(),
                Value::Int(Some(7)),
                "car".into(),
                Value::SmallInt(Some(5)),
            ],
        );
    }

    #[test]
    fn a_base_only_update_reads_the_kind_table() {
        let base = vehicle::ActiveModel {
            name: set("lorry"),
            ..keyed(3)
        };
        let unchanged = VehicleActiveModel::Truck(truck::ActiveModel::default());
        let write = Vehicle::update(base, unchanged).expect("the update is built");
        assert_sql(
            &write,
            &[
                r#"WITH "vehicle" AS (UPDATE "vehicle" SET "name" = $1"#,
                r#"WHERE "vehicle"."id" = $2 AND "vehicle"."kind" = CAST($3::text AS vehicle_kind)"#,
                VEHICLE_RETURNING,
                PROJECTION,
                CAR_JOIN,
                TRUCK_JOIN,
            ],
            vec!["lorry".into(), Value::Int(Some(3)), "truck".into()],
        );
    }

    #[test]
    fn a_kind_only_update_locks_the_base_row() {
        let write = Vehicle::update(keyed(3), truck(4, 9000)).expect("the update is built");
        assert_sql(
            &write,
            &[
                r#"WITH "vehicle" AS (SELECT "vehicle"."id", "vehicle"."kind", "vehicle"."name""#,
                r#"FROM "vehicle""#,
                r#"WHERE "vehicle"."id" = $1 AND "vehicle"."kind" = CAST($2::text AS vehicle_kind)"#,
                r#"FOR KEY SHARE)"#,
                r#", "truck" AS (UPDATE "truck" SET "axles" = $3, "payload_kg" = $4 FROM "vehicle""#,
                r#"WHERE "vehicle"."id" = "truck"."vehicle_id""#,
                TRUCK_RETURNING,
                PROJECTION,
                CAR_JOIN,
                TRUCK_JOIN,
            ],
            vec![
                Value::Int(Some(3)),
                "truck".into(),
                Value::SmallInt(Some(4)),
                Value::Int(Some(9000)),
            ],
        );

        let nothing =
            Vehicle::update(keyed(2), VehicleActiveModel::Bike).expect("the update is built");
        assert_sql(
            &nothing,
            &[
                r#"WITH "vehicle" AS (SELECT "vehicle"."id", "vehicle"."kind", "vehicle"."name""#,
                r#"FROM "vehicle""#,
                r#"WHERE "vehicle"."id" = $1 AND "vehicle"."kind" = CAST($2::text AS vehicle_kind)"#,
                r#"FOR KEY SHARE)"#,
                PROJECTION,
                CAR_JOIN,
                TRUCK_JOIN,
            ],
            vec![Value::Int(Some(2)), "bike".into()],
        );
    }

    #[test]
    fn a_change_of_kind_empties_every_other_table() {
        let write = Vehicle::change_kind(1, truck(2, 1000)).expect("the change is built");
        assert_sql(
            &write,
            &[
                r#"WITH "vehicle" AS (UPDATE "vehicle" SET "kind" = CAST($1::text AS vehicle_kind)"#,
                r#"WHERE "vehicle"."id" = $2 AND "vehicle"."kind" <> CAST($3::text AS vehicle_kind)"#,
                VEHICLE_RETURNING,
                r#", "car" AS (DELETE FROM "car" USING "vehicle""#,
                r#"WHERE "vehicle"."id" = "car"."id""#,
                CAR_RETURNING,
                r#", "truck" AS (INSERT INTO "truck" ("vehicle_id", "axles", "payload_kg")"#,
                r#"SELECT "vehicle"."id", $4, $5 FROM "vehicle""#,
                TRUCK_RETURNING,
                PROJECTION,
                r#"LEFT JOIN "car" ON FALSE"#,
                TRUCK_JOIN,
            ],
            vec![
                "truck".into(),
                Value::Int(Some(1)),
                "truck".into(),
                Value::SmallInt(Some(2)),
                Value::Int(Some(1000)),
            ],
        );

        let unit = Vehicle::change_kind(1, VehicleActiveModel::Bike).expect("the change is built");
        assert_sql(
            &unit,
            &[
                r#"WITH "vehicle" AS (UPDATE "vehicle" SET "kind" = CAST($1::text AS vehicle_kind)"#,
                r#"WHERE "vehicle"."id" = $2 AND "vehicle"."kind" <> CAST($3::text AS vehicle_kind)"#,
                VEHICLE_RETURNING,
                r#", "car" AS (DELETE FROM "car" USING "vehicle""#,
                r#"WHERE "vehicle"."id" = "car"."id""#,
                CAR_RETURNING,
                r#", "truck" AS (DELETE FROM "truck" USING "vehicle""#,
                r#"WHERE "vehicle"."id" = "truck"."vehicle_id""#,
                TRUCK_RETURNING,
                PROJECTION,
                r#"LEFT JOIN "car" ON FALSE"#,
                r#"LEFT JOIN "truck" ON FALSE"#,
            ],
            vec!["bike".into(), Value::Int(Some(1)), "bike".into()],
        );
    }

    #[test]
    fn a_composite_key_pairs_column_by_column() {
        let base = listing::ActiveModel {
            tenant_id: set(7),
            id: set(1_i64),
            title: set("loft"),
            ..Default::default()
        };
        let flat = ListingActiveModel::Flat(flat::ActiveModel {
            rooms: set(3_i16),
            ..Default::default()
        });
        let (sql, _) = Listing::insert(base, flat)
            .expect("the insert is built")
            .build();
        assert_eq!(
            sql,
            [
                r#"WITH "listing" AS (INSERT INTO "market"."listing" ("tenant_id", "id", "category", "title")"#,
                r#"VALUES ($1, $2, CAST($3::text AS market.listing_category), $4)"#,
                r#"RETURNING "listing"."tenant_id", "listing"."id", "listing"."category", "listing"."title")"#,
                r#", "flat" AS (INSERT INTO "market"."flat" ("tenant", "listing", "rooms")"#,
                r#"SELECT "listing"."tenant_id", "listing"."id", $5 FROM "listing""#,
                r#"RETURNING "flat"."tenant", "flat"."listing", "flat"."rooms")"#,
                r#"SELECT "listing"."tenant_id" AS "s0_tenant_id", "listing"."id" AS "s0_id","#,
                r#"CAST("listing"."category" AS text) AS "s0_category", "listing"."title" AS "s0_title","#,
                r#""flat"."tenant" AS "s1_tenant", "flat"."listing" AS "s1_listing", "flat"."rooms" AS "s1_rooms""#,
                r#"FROM "listing""#,
                r#"LEFT JOIN "flat" ON "listing"."tenant_id" = "flat"."tenant" AND "listing"."id" = "flat"."listing""#,
            ]
            .join(" ")
        );

        let (sql, _) = Listing::change_kind((7, 1_i64), ListingActiveModel::BuildingPlot)
            .expect("the change is built")
            .build();
        assert!(
            sql.contains(concat!(
                r#""flat" AS (DELETE FROM "market"."flat" USING "listing" "#,
                r#"WHERE "listing"."tenant_id" = "flat"."tenant" AND "listing"."id" = "flat"."listing""#,
            )),
            "{sql}"
        );
    }
}
