//! A disjoint entity's read: every base row with its kind's row, in one
//! statement.
//!
//! [`SelectDisjoint<D>`] is the base `LEFT JOIN`ed to each kind's table on the
//! key. The joins, the projection and the decode are driven by a visit over
//! the kinds rather than by a typed tuple of sources, so the number of kinds
//! is not bounded the way a [`SelectGraph`](crate::SelectGraph)'s slots are;
//! the projection and the join conditions are the graph's own, written by
//! the same writer ([`project_source`]) and computed from a [`RelationDef`] as
//! every other join is.

use core::marker::PhantomData;
use std::fmt;

use pgorm_query::{
    Condition, ConditionType, FromItem, IntoCondition, JoinType, Name, SelectStatement,
};

use super::graph::project_source;
use super::helper::join_condition;
use crate::{
    ActiveEnum, ColumnPairs, ColumnTrait, Disjoint, EntityName, EntityTrait, Iterable, KindModel,
    KindVisitor, PrimaryKeyToColumn, QueryFilter, QueryOrder, QueryTrait, RelationDef,
    RelationType, Select,
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
        let mut query = select.query;
        query.clear_selects();
        project_source::<D::Base>(&mut query, Name::new(D::Base::default()), BASE_SOURCE);
        let mut sources = BASE_SOURCE;
        for kind in D::Kind::iter() {
            D::visit_kind(
                kind,
                JoinKind {
                    query: &mut query,
                    sources: &mut sources,
                },
            );
        }
        Self {
            query,
            marker: PhantomData,
        }
    }

    /// Read only base rows of the kind `kind`.
    ///
    /// Each call is one more condition, so two kinds read nothing; several
    /// kinds are [`kinds`](Self::kinds).
    // [spec:pgorm:sem:disjoint.read+1]
    pub fn kind(self, kind: D::Kind) -> Self {
        self.filter(D::kind_column().eq(kind.to_value()))
    }

    /// Read only base rows of one of `kinds`.
    // [spec:pgorm:sem:disjoint.read+1]
    pub fn kinds<I>(self, kinds: I) -> Self
    where
        I: IntoIterator<Item = D::Kind>,
    {
        self.filter(D::kind_column().is_in(kinds.into_iter().map(|kind| kind.to_value())))
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
        let kind = M::KIND;
        self.filter(
            Condition::all()
                .add(D::kind_column().eq(kind.to_value()))
                .add(condition.into_condition()),
        )
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

/// Joins a kind's table to the base and projects it under the next prefix,
/// and does nothing for a kind without one.
///
/// `sources` counts the decoded sources projected so far after the base: the
/// decode ([`DisjointRow`](crate::DisjointRow)) counts kinds the same way, by
/// the same visit in the same order, which is what pairs each kind with its
/// prefix.
struct JoinKind<'q> {
    query: &'q mut SelectStatement,
    sources: &'q mut usize,
}

impl<D: Disjoint> KindVisitor<D> for JoinKind<'_> {
    type Output = ();

    // [spec:pgorm:sem:disjoint.read+1]
    fn table<M: KindModel<D>>(self, _: D::Kind) {
        *self.sources += 1;
        let table = FromItem::from(M::Entity::default().table_ref());
        let qualifier = Name::clone(table.qualifier());
        // A key of no column has nothing to join on. Left unjoined, the
        // projection names a table the statement does not have, which the
        // server refuses, rather than pairing every base row with every row.
        if let Some(relation) = key_relation::<D::Base, M::Entity>(table.clone()) {
            self.query
                .join(JoinType::LeftJoin, table, join_condition(relation));
        }
        project_source::<M::Entity>(self.query, qualifier, *self.sources);
    }

    fn unit(self, _: D) {}
}

/// The relation from the base to a kind's table on the key: each base key
/// column to the kind key column in the same place, typed, so a key whose
/// domain is over an enum is compared as the enum.
///
/// The two keys are of one arity, since [`KindModel`] holds the kind's key
/// types to the base's.
// [spec:pgorm:sem:disjoint.read+1]
fn key_relation<B: EntityTrait, K: EntityTrait>(to_tbl: FromItem) -> Option<RelationDef> {
    let mut pairs = B::PrimaryKey::iter()
        .map(PrimaryKeyToColumn::into_column)
        .zip(K::PrimaryKey::iter().map(PrimaryKeyToColumn::into_column));
    let (base, kind) = pairs.next()?;
    let mut columns = ColumnPairs::of_columns(base, kind);
    for (base, kind) in pairs {
        columns.push_columns(base, kind);
    }
    Some(RelationDef {
        rel_type: RelationType::HasOne,
        from_tbl: FromItem::from(B::default().table_ref()),
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
