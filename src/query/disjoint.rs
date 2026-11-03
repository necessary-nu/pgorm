//! A disjoint entity's read: every base row with its kind's row, in one
//! statement.
//!
//! [`SelectDisjoint<D>`] is the base `LEFT JOIN`ed to each kind's table on the
//! key. The joins and the projection are written from the entity's
//! [`DisjointShape`], kind by kind rather than by a typed tuple of sources,
//! so the number of kinds is not bounded the way a
//! [`SelectGraph`](crate::SelectGraph)'s slots are; the projection names its
//! columns as the graph's writer does, and the join conditions are computed
//! from a [`RelationDef`] as every other join is. A shape described at run
//! time reads through the same code.

use core::marker::PhantomData;
use std::fmt;

use pgorm_query::{
    BinOper, Condition, ConditionType, Expr, FromItem, IntoCondition, JoinType, Key, Name,
    SelectExpr, SelectStatement, SimpleExpr, TableName, Value, ValueTuple,
};

use super::graph::source_prefix;
use super::helper::join_condition;
use crate::executor::BoundaryCast;
use crate::{
    ActiveEnum, ColumnPairs, Cursor, Disjoint, DisjointColumn, DisjointShape, DisjointTable, Error,
    KindModel, QueryFilter, QueryOrder, QueryTrait, RelationDef, RelationType, Select,
    SelectUndecoded, result_column_name,
};

/// A read of a disjoint entity `D`: each base row with its kind's row, decoded
/// as the pair `(base::Model, D)`.
///
/// [`Disjoint::find`] and [`Disjoint::find_by_id`] construct it. It takes what
/// a [`Select`] of the base takes — [`filter`](QueryFilter::filter),
/// ordering, [`limit`](Self::limit) and [`offset`](Self::offset),
/// [`paginate`](Self::paginate) and [`cursor_by`](Self::cursor_by) — and
/// [`kind`](Self::kind), [`kinds`](Self::kinds) and
/// [`filter_kind`](Self::filter_kind) narrow it by kind. Its terminals are
/// [`one`](Self::one), [`all`](Self::all) and [`stream`](Self::stream).
///
/// The base's columns are projected under `s0_` and each kind table's under
/// `s{i}_`, `i` counting the kinds with a table in declaration order, the
/// names bounded to PostgreSQL's 63 bytes as a graph's sources' are
/// ([`result_column_name`](crate::result_column_name)): a kind's column never
/// shadows a base column or another kind's of the same name. No bookkeeping
/// column is selected — neither the base's generated key columns nor a kind
/// table's kind column, which no model has a field for.
///
/// ```no_run
/// # use pgorm::{entity::prelude::*, tests_cfg::{vehicle::{self, Vehicle, VehicleKind}, car}};
/// # async fn example(pool: &DatabasePool) -> Result<(), Error> {
/// let db = pool.get().await?;
///
/// let roomy: Vec<(vehicle::Model, Vehicle)> = Vehicle::find()
///     .filter_kind::<car::Model>(car::Column::Seats.gte(5))
///     .order_by_asc(vehicle::Column::Name)
///     .all(&db)
///     .await?;
/// # Ok(())
/// # }
/// ```
///
/// The projection is generated, never edited: there is no
/// [`QuerySelect`](crate::QuerySelect), so there is no `column` or
/// `select_only` to disagree with the decode.
///
/// ```compile_fail,E0599
/// # use pgorm::{entity::prelude::*, tests_cfg::vehicle::{self, Vehicle}};
/// let _ = Vehicle::find().column(vehicle::Column::Name);
/// ```
///
/// [`QueryTrait::query`] remains the raw escape hatch it is on every builder:
/// a projection edited through it stands outside the decode.
// [spec:pgorm:sem:disjoint.read+1]
pub struct SelectDisjoint<D: Disjoint> {
    pub(crate) query: SelectStatement,
    pub(crate) shape: DisjointShape,
    marker: PhantomData<fn() -> D>,
}

impl<D: Disjoint> fmt::Debug for SelectDisjoint<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SelectDisjoint")
            .field("query", &self.query)
            .finish()
    }
}

impl<D: Disjoint> Clone for SelectDisjoint<D> {
    fn clone(&self) -> Self {
        Self {
            query: self.query.clone(),
            shape: self.shape.clone(),
            marker: PhantomData,
        }
    }
}

/// The index of the base among a disjoint read's decoded sources, the
/// root's in a graph: its columns are projected under `s0_`, and each kind
/// with a table takes the next index.
// [spec:pgorm:sem:disjoint.read+1]
pub(crate) const BASE_SOURCE: usize = 0;

impl<D: Disjoint> SelectDisjoint<D> {
    /// Re-root a select of the base as a disjoint read: its `FROM` and
    /// `WHERE` kept, its projection regenerated — the base's columns under
    /// `s0_`, then each kind with a table joined and projected under the next
    /// prefix, in declaration order.
    // [spec:pgorm:sem:disjoint.read+1]
    pub(crate) fn of(select: Select<D::Base>) -> Self {
        let shape = DisjointShape::of::<D>();
        let mut query = select.query;
        query.clear_selects();
        shape.project(&mut query, &|_| KindRows::Table);
        Self {
            query,
            shape,
            marker: PhantomData,
        }
    }

    /// Read only base rows of the kind `kind`.
    ///
    /// Each call is one more condition, so two kinds read nothing; several
    /// kinds are [`kinds`](Self::kinds).
    // [spec:pgorm:sem:disjoint.read+1]
    pub fn kind(self, kind: D::Kind) -> Self {
        let condition = self.shape.kind_eq(kind.to_value());
        self.filter(condition)
    }

    /// Read only base rows of one of `kinds`.
    // [spec:pgorm:sem:disjoint.read+1]
    pub fn kinds<I>(self, kinds: I) -> Self
    where
        I: IntoIterator<Item = D::Kind>,
    {
        let condition = self
            .shape
            .kinds_in(kinds.into_iter().map(|kind| kind.to_value()));
        self.filter(condition)
    }

    /// Read only base rows of `M`'s kind, the kind whose rows are `M`s, whose
    /// row in that kind's table passes `condition`.
    ///
    /// The kind is part of the condition, so whatever `condition` makes of a
    /// row with no row in the kind's table — a row of another kind, all of
    /// whose columns there the `LEFT JOIN` leaves `NULL` — no row of another
    /// kind passes it: `car::Column::Seats.is_null()` reads cars whose seats
    /// are null, not every truck. A condition on a kind's column given to
    /// [`filter`](QueryFilter::filter) is an ordinary `WHERE`, under which
    /// those `NULL`s are compared like any other.
    // [spec:pgorm:sem:disjoint.read+1]
    pub fn filter_kind<M: KindModel<D>>(self, condition: impl IntoCondition) -> Self {
        let condition = self
            .shape
            .kind_condition(M::KIND.to_value(), condition.into_condition());
        self.filter(condition)
    }

    /// Read at most `limit` base rows, or, given `None`, remove a limit set
    /// before.
    ///
    /// Each base row is one row of the statement — a kind's table holds at
    /// most one row per key — so the limit counts base rows. A
    /// [paginator](Self::paginate)'s pages and a [cursor](Self::cursor_by)'s
    /// window replace it, as they do a [`Select`]'s.
    // [spec:pgorm:sem:disjoint.read+1]
    pub fn limit(mut self, limit: impl Into<Option<u64>>) -> Self {
        match limit.into() {
            Some(limit) => self.query.limit(limit),
            None => self.query.reset_limit(),
        };
        self
    }

    /// Skip the first `offset` base rows, or, given `None`, remove an offset
    /// set before, on the terms [`limit`](Self::limit) states.
    // [spec:pgorm:sem:disjoint.read+1]
    pub fn offset(mut self, offset: impl Into<Option<u64>>) -> Self {
        match offset.into() {
            Some(offset) => self.query.offset(offset),
            None => self.query.reset_offset(),
        };
        self
    }
}

/// Where a disjoint statement reads one kind's table from.
// [spec:pgorm:sem:disjoint.read+1]
// [spec:pgorm:sem:disjoint.write+1]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KindRows {
    /// The table as the statement's snapshot holds it, which is the table a
    /// read reads and one a write left alone.
    Table,
    /// The rows a write wrote to the table, through its data-modifying
    /// `WITH` part, which is named as the table ([`DisjointTable::written`]).
    Written,
    /// No row: the write deleted the base row's row from the table, so none
    /// of the key remains there. Its deleting part is joined `ON FALSE`,
    /// since the snapshot still holds the row it deleted.
    Deleted,
}

impl DisjointTable {
    /// The name a disjoint write gives the `WITH` part that writes the
    /// table: the table's own name, bare.
    ///
    /// Every reference to the table outside a part's target then means the
    /// rows the part wrote, so the statement's `SELECT`, the read's
    /// projection and joins ([`DisjointShape::project`]), reads the written
    /// row wherever the read reads the table, and a later part reads the base
    /// row through the same name. A part's target, and a part's own body,
    /// still name the table: PostgreSQL resolves neither to a `WITH` query. A
    /// schema-qualified table keeps its schema where it is a target and drops
    /// it here, as its qualifier in the read's projection already does.
    // [spec:pgorm:sem:disjoint.write+1]
    pub(crate) fn written(&self) -> TableName {
        TableName::Table(Name::clone(self.name()))
    }

    /// Each column of the table with the name a read gives it as the decoded
    /// source `source`, in column order: a graph's writer's name for its
    /// sources' columns.
    // [spec:pgorm:sem:disjoint.read+1]
    // [spec:pgorm:sem:query.graph.writer+4]
    fn result_columns(&self, source: usize) -> impl Iterator<Item = (&DisjointColumn, String)> {
        let prefix = source_prefix(source);
        self.columns.iter().map(move |column| {
            (
                column,
                result_column_name(&prefix, &column.name.to_string()),
            )
        })
    }
}

/// Project every column of `table` onto `query` under the prefix of the
/// decoded source `index`, each read through its `select_as`.
// [spec:pgorm:sem:disjoint.read+1]
fn project_table(
    query: &mut SelectStatement,
    table: &DisjointTable,
    qualifier: &Name,
    index: usize,
) {
    for (column, alias) in table.result_columns(index) {
        let expr = Expr::col((Name::clone(qualifier), Name::clone(&column.name)));
        query.expr(SelectExpr::new_as(
            column.casts.select(expr),
            Name::runtime(alias),
        ));
    }
}

/// Each key column of `table` compared with the value `key` gives it, in key
/// order, as `EntityTrait::find_by_id` compares a key: the column and the
/// value each under the column's casts.
// [spec:pgorm:sem:disjoint.read+1]
// [spec:pgorm:sem:disjoint.write+1]
pub(crate) fn key_filter(table: &DisjointTable, key: impl IntoIterator<Item = Value>) -> Condition {
    let mut condition = Condition::all();
    for (column, value) in table.key().zip(key) {
        let compared = column.casts.compared_column(Expr::col((
            Name::clone(table.name()),
            Name::clone(&column.name),
        )));
        condition = condition.add(compared.binary(BinOper::Equal, column.casts.compared(value)));
    }
    condition
}

impl DisjointShape {
    /// The condition narrowing a read to the base row whose key `key` gives,
    /// in key order, as [`Disjoint::find_by_id`] narrows it; a key of another
    /// arity than the base's is refused as [`Error::Type`].
    // [spec:pgorm:sem:disjoint.read+1]
    // [spec:pgorm:req:bindings.disjoint+1]
    pub fn key_is(&self, key: Vec<Value>) -> Result<Condition, Error> {
        self.key_arity(key.len())?;
        Ok(key_filter(&self.base, key))
    }

    /// Refuse a key of `arity` values unless the base's key has as many
    /// columns.
    pub(crate) fn key_arity(&self, arity: usize) -> Result<(), Error> {
        if arity == self.base.key.len() {
            return Ok(());
        }
        Err(Error::Type(format!(
            "the key of `{}` is {} value(s), not {arity}",
            self.base.label,
            self.base.key.len(),
        )))
    }

    /// Every base row with its kind's row, one row of the statement each: the
    /// base's columns under `s0_`, then each kind with a table `LEFT JOIN`ed
    /// on the key and projected under the next prefix, in declaration order.
    ///
    /// The statement [`Disjoint::find`] builds, for a shape described at run
    /// time; [`kind_is`](Self::kind_is), [`kind_in`](Self::kind_in) and
    /// [`filter_kind`](Self::filter_kind) give the conditions that narrow it
    /// by kind.
    // [spec:pgorm:sem:disjoint.read+1]
    // [spec:pgorm:req:bindings.disjoint+1]
    pub fn find(&self) -> SelectStatement {
        let mut query = SelectStatement::new();
        query.from(self.base.table.clone());
        self.project(&mut query, &|_| KindRows::Table);
        query
    }

    /// Project a disjoint read onto `query`, whose `FROM` reads the base: the
    /// base's columns under `s0_`, then each kind with a table joined from
    /// where `rows` says for the kind at that index and projected under the
    /// next prefix, in declaration order.
    // [spec:pgorm:sem:disjoint.read+1]
    // [spec:pgorm:sem:disjoint.write+1]
    pub(crate) fn project(&self, query: &mut SelectStatement, rows: &dyn Fn(usize) -> KindRows) {
        project_table(query, &self.base, self.base.name(), BASE_SOURCE);
        let mut sources = BASE_SOURCE;
        for (index, kind) in self.kinds.iter().enumerate() {
            let Some(table) = &kind.table else { continue };
            sources += 1;
            let rows = rows(index);
            let from = match rows {
                KindRows::Table => FromItem::from(table.table.clone()),
                KindRows::Written | KindRows::Deleted => FromItem::from(table.written()),
            };
            let qualifier = Name::clone(from.qualifier());
            match rows {
                KindRows::Deleted => {
                    query.join(JoinType::LeftJoin, from, SimpleExpr::Constant(false.into()));
                }
                // A key of no column has nothing to join on. Left unjoined,
                // the projection names a table the statement does not have,
                // which the server refuses, rather than pairing every base
                // row with every row.
                KindRows::Table | KindRows::Written => {
                    if let Some(relation) = self.key_relation(table, from.clone()) {
                        query.join(JoinType::LeftJoin, from, join_condition(relation));
                    }
                }
            }
            project_table(query, table, &qualifier, sources);
        }
    }

    /// The relation from the base to `table`, read as `to_tbl`, on the key:
    /// each base key column to `table`'s key column in the same place, each
    /// side cast to the enum its domain is over where it is one.
    // [spec:pgorm:sem:disjoint.read+1]
    // [spec:pgorm:sem:disjoint.write+1]
    pub(crate) fn key_relation(
        &self,
        table: &DisjointTable,
        to_tbl: FromItem,
    ) -> Option<RelationDef> {
        let mut pairs = self.key_pairs(table);
        let (base, kind, compared) = pairs.next()?;
        let mut columns =
            ColumnPairs::of_compared(Name::clone(&base.name), Name::clone(&kind.name), compared);
        for (base, kind, compared) in pairs {
            columns.push_compared(Name::clone(&base.name), Name::clone(&kind.name), compared);
        }
        Some(RelationDef {
            rel_type: RelationType::HasOne,
            from_tbl: FromItem::from(self.base.table.clone()),
            to_tbl,
            columns,
            is_owner: true,
            on_delete: None,
            on_update: None,
            on_condition: None,
            fk_name: None,
            enforcement: None,
            deferrability: None,
            condition_type: ConditionType::All,
        })
    }

    /// The names a read of this shape gives the columns of the decoded
    /// source `source`, in column order, or `None` for an index no source
    /// has: the base is source 0, projected under
    /// [`source_prefix`](crate::source_prefix)`(0)`, and each kind with a
    /// table the index [`sources`](Self::sources) pairs it with.
    // [spec:pgorm:sem:disjoint.read+1]
    // [spec:pgorm:req:bindings.disjoint+1]
    pub fn result_names(&self, source: usize) -> Option<Vec<String>> {
        let table = match source {
            BASE_SOURCE => &self.base,
            _ => self.sources().find(|(index, ..)| *index == source)?.2,
        };
        Some(table.result_columns(source).map(|(_, name)| name).collect())
    }

    /// The base's kind column as a comparison reads it.
    fn kind_compared(&self) -> Expr {
        let column = self.kind_column();
        column.casts.compared_column(Expr::col((
            Name::clone(self.base.name()),
            Name::clone(&column.name),
        )))
    }

    /// The kind column compared with `value` by `op`, as a column's value
    /// predicate compares it.
    pub(crate) fn kind_compare(&self, op: BinOper, value: String) -> SimpleExpr {
        let compared = self.kind_column().casts.compared(value.into());
        self.kind_compared().binary(op, compared)
    }

    pub(crate) fn kind_eq(&self, value: String) -> SimpleExpr {
        self.kind_compare(BinOper::Equal, value)
    }

    /// The kind column among `values`.
    pub(crate) fn kinds_in(&self, values: impl IntoIterator<Item = String>) -> SimpleExpr {
        let casts = &self.kind_column().casts;
        self.kind_compared()
            .is_in(values.into_iter().map(|value| casts.compared(value.into())))
    }

    pub(crate) fn kind_condition(&self, value: String, condition: Condition) -> Condition {
        Condition::all().add(self.kind_eq(value)).add(condition)
    }

    /// The condition narrowing a read to base rows of the kind `kind`, which
    /// [`SelectDisjoint::kind`] adds; a value no kind has is refused as
    /// [`Error::Type`].
    // [spec:pgorm:sem:disjoint.read+1]
    // [spec:pgorm:req:bindings.disjoint+1]
    pub fn kind_is(&self, kind: &str) -> Result<SimpleExpr, Error> {
        let (_, kind) = self.kind(kind)?;
        Ok(self.kind_eq(kind.value.clone()))
    }

    /// The condition narrowing a read to base rows of one of `kinds`, which
    /// [`SelectDisjoint::kinds`] adds; a value no kind has is refused as
    /// [`Error::Type`].
    // [spec:pgorm:sem:disjoint.read+1]
    // [spec:pgorm:req:bindings.disjoint+1]
    pub fn kind_in<S: AsRef<str>>(&self, kinds: &[S]) -> Result<SimpleExpr, Error> {
        let values = kinds
            .iter()
            .map(|kind| self.kind(kind.as_ref()).map(|(_, kind)| kind.value.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self.kinds_in(values))
    }

    /// The condition narrowing a read to base rows of the kind `kind` whose
    /// row in its table passes `condition`, which
    /// [`SelectDisjoint::filter_kind`] adds: the kind is part of it, so a
    /// condition a missing row satisfies reads no row of another kind. A
    /// value no kind has is refused as [`Error::Type`].
    // [spec:pgorm:sem:disjoint.read+1]
    pub fn filter_kind(
        &self,
        kind: &str,
        condition: impl IntoCondition,
    ) -> Result<Condition, Error> {
        let (_, kind) = self.kind(kind)?;
        Ok(self.kind_condition(kind.value.clone(), condition.into_condition()))
    }

    /// The cast a keyset boundary on the base's column `column` takes: the
    /// column's own, or none for a name no base column has.
    pub(crate) fn boundary_cast(&self, column: &str) -> BoundaryCast {
        match self.base.column(column) {
            Some((_, column)) => column.casts.boundary(),
            None => BoundaryCast::identity(),
        }
    }

    /// A keyset cursor over `query`, a read of this shape, ordered by the
    /// base's columns `columns`, each boundary value bound under its column's
    /// casts: the cursor [`SelectDisjoint::cursor_by`] makes, for a shape
    /// described at run time. A name no base column has is refused as
    /// [`Error::Type`].
    ///
    /// Its [`statement`](Cursor::statement) is what a page of it sends.
    // [spec:pgorm:sem:disjoint.read+1]
    // [spec:pgorm:req:bindings.disjoint+1]
    pub fn cursor_by<S: AsRef<str>>(
        &self,
        query: SelectStatement,
        columns: &[S],
    ) -> Result<Cursor<SelectUndecoded, ValueTuple>, Error> {
        let mut names = Vec::with_capacity(columns.len());
        for column in columns {
            let Some((_, column)) = self.base.column(column.as_ref()) else {
                return Err(Error::Type(format!(
                    "`{}` has no column `{}` to order a cursor by",
                    self.base.label,
                    column.as_ref()
                )));
            };
            names.push(Name::clone(&column.name));
        }
        if names.is_empty() {
            return Err(Error::Type(
                "a cursor is ordered by at least one column".to_owned(),
            ));
        }
        Ok(
            Cursor::new(query, Name::clone(self.base.name()), Key::from(names))
                .with_casts_by(|column| self.boundary_cast(column)),
        )
    }
}

// [spec:pgorm:sem:disjoint.read+1]
impl<D: Disjoint> QueryFilter for SelectDisjoint<D> {
    type QueryStatement = SelectStatement;

    fn query(&mut self) -> &mut SelectStatement {
        &mut self.query
    }
}

// [spec:pgorm:sem:disjoint.read+1]
impl<D: Disjoint> QueryOrder for SelectDisjoint<D> {
    type QueryStatement = SelectStatement;

    fn query(&mut self) -> &mut SelectStatement {
        &mut self.query
    }
}

// [spec:pgorm:sem:disjoint.read+1]
impl<D: Disjoint> QueryTrait for SelectDisjoint<D> {
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

// [spec:pgorm:sem:disjoint.read+1/test]    the read's one statement: the base projected under `s0_` and each kind's table LEFT JOINed on the key and projected under its own prefix, no bookkeeping column selected; `find_by_id` keys it, composite and schema-qualified keys included; `kind`, `kinds` and `filter_kind` narrow it by kind, the last with the kind inside the condition; `limit` and `offset` bind on it
#[cfg(test)]
mod tests {
    use crate::tests_cfg::{
        car,
        listing::{Listing, ListingKind},
        truck,
        vehicle::{self, Vehicle, VehicleKind},
    };
    use crate::{ColumnTrait, Disjoint, QueryFilter, QueryOrder, QueryTrait};
    use pgorm_query::{Value, Values};
    use pretty_assertions::assert_eq;

    const VEHICLE_READ: [&str; 6] = [
        r#"SELECT "vehicle"."id" AS "s0_id", CAST("vehicle"."kind" AS text) AS "s0_kind", "vehicle"."name" AS "s0_name","#,
        r#""car"."id" AS "s1_id", "car"."seats" AS "s1_seats","#,
        r#""truck"."vehicle_id" AS "s2_vehicle_id", "truck"."axles" AS "s2_axles", "truck"."payload_kg" AS "s2_payload_kg""#,
        r#"FROM "vehicle""#,
        r#"LEFT JOIN "car" ON "vehicle"."id" = "car"."id""#,
        r#"LEFT JOIN "truck" ON "vehicle"."id" = "truck"."vehicle_id""#,
    ];

    #[track_caller]
    fn assert_sql(actual: String, parts: &[&str]) {
        assert_eq!(actual, parts.join(" "));
    }

    #[test]
    fn a_read_joins_every_kind_table_once() {
        assert_sql(Vehicle::find().build().0, &VEHICLE_READ);
    }

    #[test]
    fn find_by_id_keys_the_base_row() {
        let (sql, values) = Vehicle::find_by_id(7).build();
        assert_sql(
            sql,
            &[&VEHICLE_READ.join(" "), r#"WHERE "vehicle"."id" = $1"#],
        );
        assert_eq!(values, Values(vec![Value::Int(Some(7))]));
    }

    #[test]
    fn a_composite_key_joins_column_by_column() {
        let (sql, values) = Listing::find_by_id((7, 1_i64)).build();
        assert_sql(
            sql,
            &[
                r#"SELECT "listing"."tenant_id" AS "s0_tenant_id", "listing"."id" AS "s0_id","#,
                r#"CAST("listing"."category" AS text) AS "s0_category", "listing"."title" AS "s0_title","#,
                r#""flat"."tenant" AS "s1_tenant", "flat"."listing" AS "s1_listing", "flat"."rooms" AS "s1_rooms""#,
                r#"FROM "market"."listing""#,
                r#"LEFT JOIN "market"."flat" ON "listing"."tenant_id" = "flat"."tenant" AND "listing"."id" = "flat"."listing""#,
                r#"WHERE "listing"."tenant_id" = $1 AND "listing"."id" = $2"#,
            ],
        );
        assert_eq!(
            values,
            Values(vec![Value::Int(Some(7)), Value::BigInt(Some(1))])
        );
    }

    #[test]
    fn kinds_narrow_on_the_base_kind_column() {
        let (sql, values) = Vehicle::find().kind(VehicleKind::Car).build();
        assert!(
            sql.ends_with(r#"WHERE "vehicle"."kind" = CAST($1::text AS vehicle_kind)"#),
            "{sql}"
        );
        assert_eq!(values, Values(vec!["car".into()]));

        let sql = Listing::find()
            .kinds([ListingKind::Flat, ListingKind::BuildingPlot])
            .build()
            .0;
        assert!(
            sql.ends_with(r#"WHERE "listing"."category" IN (CAST($1::text AS market.listing_category), CAST($2::text AS market.listing_category))"#),
            "{sql}"
        );
    }

    #[test]
    fn filter_kind_puts_the_kind_in_the_condition() {
        let sql = Vehicle::find()
            .filter(vehicle::Column::Name.ne("lorry"))
            .filter_kind::<truck::Model>(truck::Column::Axles.is_null())
            .build()
            .0;
        assert!(
            sql.ends_with(
                r#"WHERE "vehicle"."name" <> $1 AND "vehicle"."kind" = CAST($2::text AS vehicle_kind) AND "truck"."axles" IS NULL"#
            ),
            "{sql}"
        );
    }

    #[test]
    fn limit_and_offset_bind_on_the_read() {
        let (sql, values) = Vehicle::find()
            .filter_kind::<car::Model>(car::Column::Seats.gte(4))
            .order_by_asc(vehicle::Column::Name)
            .limit(2)
            .offset(3)
            .build();
        assert!(
            sql.ends_with(r#"ORDER BY "vehicle"."name" ASC LIMIT $3 OFFSET $4"#),
            "{sql}"
        );
        assert_eq!(
            values.0[2..],
            [Value::BigUnsigned(Some(2)), Value::BigUnsigned(Some(3))]
        );

        let unsliced = Vehicle::find().limit(2).offset(3).limit(None).offset(None);
        assert_eq!(unsliced.build(), Vehicle::find().build());
    }
}
