//! A disjoint entity's writes: insert, update and change of kind, each one
//! statement whose parts are data-modifying `WITH` clauses.
//!
//! Every part writing a table is named as that table ([`written_table`]), so
//! the statement's `SELECT` is the read's own projection and joins
//! ([`project_disjoint`]) reading each written table through the rows its
//! part wrote, and decodes through the read's decode. Every part writing a
//! kind's table reads the base row's key from the base's part, and that is
//! the only order PostgreSQL holds the parts to: they all see one snapshot,
//! and a part reading another's `RETURNING` runs after it. A base part that
//! touches no row therefore leaves every other part touching none, and the
//! statement returning no row.

use core::marker::PhantomData;
use std::fmt;

use pgorm_query::{
    CommonTableExpression, Condition, Expr, FromItem, InsertStatement, IntoName,
    IntoSubQueryStatement, LockType, Name, Query, ReturningClause, SelectStatement, WithClause,
};

use super::disjoint::{KindRows, key_pairs, key_relation, project_disjoint, written_table};
use super::helper::join_condition;
use super::insert::saved_values;
use crate::{
    ActiveEnum, ActiveKindVisitor, ActiveModelTrait, ActiveValue, ColumnTrait, Disjoint,
    EntityName, EntityTrait, Error, Iterable, KindModel, KindVisitor, PrimaryKeyToColumn,
    QueryFilter, QueryTrait, StaticName, Update, UpdateMany, UpdateOne, Value, table_label,
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
    /// The statement whose `WITH` holds the base's part and then `kinds`, and
    /// whose `SELECT` reads the base row through the base's part and each
    /// kind's table from where `rows` says.
    // [spec:pgorm:sem:disjoint.write+1]
    pub(crate) fn of(
        base: CommonTableExpression,
        kinds: impl IntoIterator<Item = CommonTableExpression>,
        rows: &dyn Fn(D::Kind) -> KindRows,
    ) -> Self {
        let mut with = WithClause::new(base);
        for part in kinds {
            with.cte(part);
        }
        let mut query = SelectStatement::new();
        query.from(written_table::<D::Base>());
        project_disjoint::<D>(&mut query, rows);
        query.with(with);
        Self {
            query,
            marker: PhantomData,
        }
    }
}

/// The part of a write that writes `E`'s table, named as the table.
// [spec:pgorm:sem:disjoint.write+1]
pub(crate) fn written_part<E: EntityTrait>(
    statement: impl IntoSubQueryStatement,
) -> CommonTableExpression {
    CommonTableExpression::new(Name::clone(written_table::<E>().table()), statement)
}

/// Every column of `E`, in iteration order, qualified by its table's bare
/// name: what a part returns, as stored, for the statement's `SELECT` to cast
/// as the read casts it.
///
/// Qualified because a part that also reads another part (`UPDATE .. FROM`,
/// `DELETE .. USING`) sees two relations whose columns may share names.
fn qualified_columns<E: EntityTrait>() -> Vec<(Name, E::Column)> {
    let table = Name::clone(E::default().table_ref().table());
    E::Column::iter()
        .map(|column| (Name::clone(&table), column))
        .collect()
}

/// The `RETURNING` of a part writing `E`'s table.
// [spec:pgorm:sem:disjoint.write+1]
pub(crate) fn returned<E: EntityTrait>() -> ReturningClause {
    Query::returning().columns(qualified_columns::<E>())
}

/// The base's part of an update: the `UPDATE` of its set columns, narrowed
/// to its key and its kind, or, when it sets none, the `SELECT` of the row so
/// narrowed `FOR KEY SHARE`.
///
/// Either way the base row is locked before any kind part reads it: the
/// `UPDATE` by its row lock, the `SELECT` by `FOR KEY SHARE`, which a change
/// of the row's kind (it writes the kind column, in the base's `UNIQUE` key)
/// and a delete both wait on. Under `READ COMMITTED` a part blocked behind
/// one re-checks its `WHERE` against the row that committed, so a row whose
/// kind changed meanwhile is no row, `Error::RecordNotFound`, rather than a
/// base row of the old kind returned beside a kind row the change deleted.
// [spec:pgorm:sem:disjoint.write+1]
pub(crate) fn updated_base<B: EntityTrait>(
    update: UpdateOne<B::ActiveModel>,
) -> CommonTableExpression {
    let mut query = update.into_query();
    if !query.get_values().is_empty() {
        query.returning(returned::<B>());
        return written_part::<B>(query);
    }
    let mut guard = SelectStatement::new();
    guard
        .columns(qualified_columns::<B>())
        .from(B::default().table_ref())
        .lock(LockType::KeyShare);
    if let Some(condition) = query.where_condition() {
        guard.cond_where(condition.clone());
    }
    written_part::<B>(guard)
}

/// The condition pairing a row of `K`'s table with the base's part on the
/// key, column by column.
fn key_condition<B: EntityTrait, K: EntityTrait>() -> Result<Condition, Error> {
    key_relation::<B, K>(FromItem::from(K::default().table_ref()))
        .map(join_condition)
        .ok_or_else(|| {
            Error::Type(format!(
                "`{}` has no primary key column to key its kinds' rows by",
                table_label(&B::default())
            ))
        })
}

/// Which values of an `ActiveModel` a write would write: an insert writes
/// `Set` and `Unchanged` values alike, an update `Set` ones alone.
// [spec:pgorm:sem:disjoint.write+1]
#[derive(Clone, Copy, Debug)]
pub(crate) enum Writes {
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

/// Refuse, as [`Error::Type`] naming the table and column, a value in
/// `column` of `active` that the write `writes` would write, where the
/// column's value comes from elsewhere for the reason `why` gives.
// [spec:pgorm:sem:disjoint.write+1]
pub(crate) fn unwritten<A: ActiveModelTrait>(
    active: &A,
    column: <A::Entity as EntityTrait>::Column,
    writes: Writes,
    why: &str,
) -> Result<(), Error> {
    if !writes.writes(&active.get(column)?) {
        return Ok(());
    }
    Err(Error::Type(format!(
        "`{}`.`{}` {}: {why}",
        table_label(&A::Entity::default()),
        column.as_str(),
        match writes {
            Writes::Insert => "holds a value",
            Writes::Update => "is `Set`",
        },
    )))
}

/// Refuse a value of a kind row's key that `writes` would write: the row's
/// key is its base row's.
fn unwritten_key<D: Disjoint, M: KindModel<D>>(
    active: &<M::Entity as EntityTrait>::ActiveModel,
    kind: D::Kind,
    writes: Writes,
) -> Result<(), Error> {
    for key in <M::Entity as EntityTrait>::PrimaryKey::iter() {
        unwritten(
            active,
            key.into_column(),
            writes,
            &format!(
                "it is the key of the kind `{}`, whose row takes its key from its base row, so \
                 its `ActiveModel` leaves it `NotSet`",
                kind.to_value()
            ),
        )?;
    }
    Ok(())
}

/// The part inserting the kind's row, its key read from the base's part, or
/// none for a kind without a table.
// [spec:pgorm:sem:disjoint.write+1]
pub(crate) struct InsertKind;

impl<D: Disjoint> ActiveKindVisitor<D> for InsertKind {
    type Output = Result<Option<CommonTableExpression>, Error>;

    fn table<M: KindModel<D>>(
        self,
        kind: D::Kind,
        active: <M::Entity as EntityTrait>::ActiveModel,
    ) -> Self::Output {
        unwritten_key::<D, M>(&active, kind, Writes::Insert)?;
        let base = written_table::<D::Base>();
        let mut columns = Vec::new();
        let mut row = SelectStatement::new();
        for (base_key, kind_key) in key_pairs::<D::Base, M::Entity>() {
            columns.push(kind_key.into_name());
            row.expr(Expr::col((Name::clone(base.table()), base_key)));
        }
        for (column, value) in saved_values(active) {
            if let Some(value) = value {
                columns.push(column.into_name());
                row.expr(value);
            }
        }
        row.from(base);
        let mut insert = InsertStatement::new();
        insert
            .into_table(M::Entity::default().table_ref())
            .columns(columns)
            .select_from(row)?
            .returning(returned::<M::Entity>());
        Ok(Some(written_part::<M::Entity>(insert)))
    }

    fn unit(self, _: D::Kind) -> Self::Output {
        Ok(None)
    }
}

/// The part updating the set columns of the kind's row, found by the base
/// part's key, or none for a kind without a table or a row setting nothing.
// [spec:pgorm:sem:disjoint.write+1]
pub(crate) struct UpdateKind;

impl<D: Disjoint> ActiveKindVisitor<D> for UpdateKind {
    type Output = Result<Option<CommonTableExpression>, Error>;

    fn table<M: KindModel<D>>(
        self,
        kind: D::Kind,
        active: <M::Entity as EntityTrait>::ActiveModel,
    ) -> Self::Output {
        unwritten_key::<D, M>(&active, kind, Writes::Update)?;
        let update = Update::many(M::Entity::default()).set(active);
        if update.as_query().get_values().is_empty() {
            return Ok(None);
        }
        let mut query = update
            .from(written_table::<D::Base>())
            .filter(key_condition::<D::Base, M::Entity>()?)
            .into_query();
        query.returning(returned::<M::Entity>());
        Ok(Some(written_part::<M::Entity>(query)))
    }

    fn unit(self, _: D::Kind) -> Self::Output {
        Ok(None)
    }
}

/// The part deleting the base row's row from a kind's table, or none for a
/// kind without one.
// [spec:pgorm:sem:disjoint.write+1]
pub(crate) struct DeleteKind;

impl<D: Disjoint> KindVisitor<D> for DeleteKind {
    type Output = Result<Option<CommonTableExpression>, Error>;

    fn table<M: KindModel<D>>(self, _: D::Kind) -> Self::Output {
        let mut delete = Query::delete();
        delete
            .from_table(M::Entity::default().table_ref())
            .using(written_table::<D::Base>())
            .cond_where(key_condition::<D::Base, M::Entity>()?)
            .returning(returned::<M::Entity>());
        Ok(Some(written_part::<M::Entity>(delete)))
    }

    fn unit(self, _: D) -> Self::Output {
        Ok(None)
    }
}

/// The base's part of a change of kind: its kind set to `kind`, narrowed to
/// `update`'s key and to a row of another kind.
// [spec:pgorm:sem:disjoint.write+1]
pub(crate) fn rekinded_base<D: Disjoint>(
    update: UpdateMany<D::Base>,
    kind: D::Kind,
) -> CommonTableExpression {
    let column = D::kind_column();
    let mut query = update
        .col_expr(column, column.save_as(Expr::val(kind.to_value())))
        .filter(column.ne(kind.to_value()))
        .into_query();
    query.returning(returned::<D::Base>());
    written_part::<D::Base>(query)
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
