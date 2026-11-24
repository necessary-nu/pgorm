use crate::query::helper::unpack_table_name;
use crate::{
    ColumnPairs, EntityTrait, NoContext, QuerySelect, Related, RelationDef, ScopeContext,
    ScopeError, ScopedIn, Select,
};
pub use pgorm_query::JoinType;
use pgorm_query::{Condition, ConditionType, Expr, ExprTrait, Name, TypeName};

// [spec:pgorm:sem:query.build.join+5]
impl<E> Select<E>
where
    E: EntityTrait,
{
    /// Left Join with a Related Entity.
    pub fn left_join<R>(self, _: R) -> Self
    where
        R: EntityTrait + ScopedIn<NoContext>,
        E: Related<R>,
    {
        join_related(self, None, JoinType::LeftJoin, E::to(), E::via())
    }

    /// [`left_join`](Select::left_join) under the context `cx`: the joined
    /// table's scopes, and a junction's, ANDed into their `ON`s.
    // [spec:pgorm:sem:scope.join]
    pub fn left_join_for<Cx, R>(self, cx: &Cx, _: R) -> Self
    where
        Cx: ScopeContext,
        R: ScopedIn<Cx>,
        E: Related<R>,
    {
        join_related(self, Some(cx), JoinType::LeftJoin, E::to(), E::via())
    }

    /// Right Join with a Related Entity.
    pub fn right_join<R>(self, _: R) -> Self
    where
        R: EntityTrait + ScopedIn<NoContext>,
        E: Related<R>,
    {
        join_related(self, None, JoinType::RightJoin, E::to(), E::via())
    }

    /// [`right_join`](Select::right_join) under the context `cx`, as
    /// [`left_join_for`](Select::left_join_for) scopes it.
    // [spec:pgorm:sem:scope.join]
    pub fn right_join_for<Cx, R>(self, cx: &Cx, _: R) -> Self
    where
        Cx: ScopeContext,
        R: ScopedIn<Cx>,
        E: Related<R>,
    {
        join_related(self, Some(cx), JoinType::RightJoin, E::to(), E::via())
    }

    /// Inner Join with a Related Entity.
    pub fn inner_join<R>(self, _: R) -> Self
    where
        R: EntityTrait + ScopedIn<NoContext>,
        E: Related<R>,
    {
        join_related(self, None, JoinType::InnerJoin, E::to(), E::via())
    }

    /// [`inner_join`](Select::inner_join) under the context `cx`, as
    /// [`left_join_for`](Select::left_join_for) scopes it.
    // [spec:pgorm:sem:scope.join]
    pub fn inner_join_for<Cx, R>(self, cx: &Cx, _: R) -> Self
    where
        Cx: ScopeContext,
        R: ScopedIn<Cx>,
        E: Related<R>,
    {
        join_related(self, Some(cx), JoinType::InnerJoin, E::to(), E::via())
    }

    /// Join with an Entity Related to me.
    pub fn reverse_join<R>(self, _: R) -> Self
    where
        R: EntityTrait + Related<E> + ScopedIn<NoContext>,
    {
        join_side(self, None, JoinType::InnerJoin, R::to(), Joined::From)
    }

    /// [`reverse_join`](Select::reverse_join) under the context `cx`, the
    /// joined table's scopes ANDed into its `ON`.
    // [spec:pgorm:sem:scope.join]
    pub fn reverse_join_for<Cx, R>(self, cx: &Cx, _: R) -> Self
    where
        Cx: ScopeContext,
        R: Related<E> + ScopedIn<Cx>,
    {
        join_side(self, Some(cx), JoinType::InnerJoin, R::to(), Joined::From)
    }
}

/// `query` joined to a related entity along `rel`, through `via` first where
/// a junction stands between, each hop scoped by `cx`.
// [spec:pgorm:sem:query.build.join+5]
// [spec:pgorm:sem:scope.join]
pub(crate) fn join_related<Q: QuerySelect>(
    mut query: Q,
    cx: Option<&dyn ScopeContext>,
    join: JoinType,
    rel: RelationDef,
    via: Option<RelationDef>,
) -> Q {
    if let Some(via) = via {
        query = join_side(query, cx, join, via, Joined::To);
    }
    join_side(query, cx, join, rel, Joined::To)
}

/// [`join_related`] from the related side: `rel` and then `via` reversed into
/// the statement, each hop scoped by `cx`.
// [spec:pgorm:sem:query.build.join+5]
// [spec:pgorm:sem:scope.join]
pub(crate) fn join_related_rev<Q: QuerySelect>(
    mut query: Q,
    cx: Option<&dyn ScopeContext>,
    join: JoinType,
    rel: RelationDef,
    via: Option<RelationDef>,
) -> Q {
    query = join_side(query, cx, join, rel, Joined::From);
    if let Some(via) = via {
        query = join_side(query, cx, join, via, Joined::From);
    }
    query
}

/// Join `rel`'s `joined` side under the relation's condition and, where that
/// side's table is scoped, its scopes under `cx` ANDed into the `ON`: the one
/// join every relation join of a select goes through. A scoped side with no
/// `cx`, or one `cx` does not hold the scopes of, is joined unscoped and the
/// refusal held, so the statement never runs.
// [spec:pgorm:sem:query.build.join+5]
// [spec:pgorm:sem:scope.join]
pub(crate) fn join_side<Q: QuerySelect>(
    mut query: Q,
    cx: Option<&dyn ScopeContext>,
    join: JoinType,
    rel: RelationDef,
    joined: Joined,
) -> Q {
    let (table, scopes) = match joined {
        Joined::To => (rel.to_tbl.clone(), rel.to_scopes),
        Joined::From => (rel.from_tbl.clone(), rel.from_scopes),
    };
    let mut condition = join_condition(rel);
    if let Some(scopes) = scopes {
        let qualifier = Name::clone(table.qualifier());
        let scoped = match cx {
            Some(cx) => scopes.condition(cx, qualifier),
            None => Err(ScopeError::Unscoped {
                table: unpack_table_name(&table).table().to_string(),
                scopes: scopes.listed(),
            }),
        };
        match scoped {
            Ok(Some(scope)) => condition = Condition::all().add(condition).add(scope),
            Ok(None) => {}
            Err(refusal) => query.refuse(refusal),
        }
    }
    query.query().join(join, table, condition);
    query
}

/// Which side of a relation a join adds to the statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Joined {
    /// The side the relation points at, `to_tbl`.
    To,
    /// The side it points from, `from_tbl`: a reversed join.
    From,
}

// [spec:pgorm:sem:query.build.join+5]
pub(crate) fn join_condition(mut rel: RelationDef) -> Condition {
    // Use table alias (if any) to construct the join condition
    let from_tbl = Name::clone(rel.from_tbl.qualifier());
    let to_tbl = Name::clone(rel.to_tbl.qualifier());
    let mut condition = match rel.condition_type {
        ConditionType::All => Condition::all(),
        ConditionType::Any => Condition::any(),
    };

    condition = condition.add(join_tbl_on_condition(
        Name::clone(&from_tbl),
        Name::clone(&to_tbl),
        rel.columns,
    ));
    if let Some(f) = rel.on_condition.take() {
        condition = condition.add(f(from_tbl, to_tbl));
    }

    condition
}

// [spec:pgorm:sem:query.build.join+5]
// [spec:pgorm:sem:entity.traits.column.enum-cast+18]
fn join_tbl_on_condition(from_tbl: Name, to_tbl: Name, columns: ColumnPairs) -> Condition {
    let mut cond = Condition::all();
    for ((owner_key, foreign_key), compared) in columns.compared_pairs() {
        let from = compared_side(
            Expr::col((Name::clone(&from_tbl), Name::clone(owner_key))),
            &compared.from,
        );
        let to = compared_side(
            Expr::col((Name::clone(&to_tbl), Name::clone(foreign_key))),
            &compared.to,
        );
        cond = cond.add(from.eq(to));
    }
    // A temporal relation's periods meet where they overlap: a row is joined
    // to every row whose period shares a moment with its own.
    if let Some((from_period, to_period)) = columns.period().cloned() {
        cond =
            cond.add(Expr::col((from_tbl, from_period)).overlaps(Expr::col((to_tbl, to_period))));
    }
    cond
}

/// One side of a join pair as the equality compares it: cast to the enum when
/// its column's domain is over one, which is the only way PostgreSQL compares
/// such a column with anything.
// [spec:pgorm:sem:entity.traits.column.enum-cast+18]
fn compared_side(column: Expr, enum_type: &Option<TypeName>) -> Expr {
    match enum_type {
        Some(enum_type) => Expr::expr(column.cast_as_type(enum_type.clone())),
        None => column,
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_cfg::{cake, cake_filling, cake_filling_price, entity_linked, filling, fruit};
    use crate::{
        ColumnTrait, EntityTrait, ModelTrait, QueryFilter, QuerySelect, QueryTrait, RelationTrait,
    };
    use pgorm_query::{ConditionType, Expr, ExprTrait, IntoCondition, JoinType, Name, alias};
    use pretty_assertions::assert_eq;

    #[test]
    fn join_1() {
        assert_eq!(
            cake::Entity::find()
                .left_join(fruit::Entity)
                .as_query()
                .to_string(),
            [
                r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
                r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id""#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_2() {
        assert_eq!(
            cake::Entity::find()
                .inner_join(fruit::Entity)
                .filter(fruit::Column::Name.contains("cherry"))
                .as_query()
                .to_string(),
            [
                r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
                r#"INNER JOIN "fruit" ON "cake"."id" = "fruit"."cake_id""#,
                r#"WHERE "fruit"."name" LIKE '%cherry%'"#
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_3() {
        assert_eq!(
            fruit::Entity::find()
                .reverse_join(cake::Entity)
                .as_query()
                .to_string(),
            [
                r#"SELECT "fruit"."id", "fruit"."name", "fruit"."cake_id" FROM "fruit""#,
                r#"INNER JOIN "cake" ON "cake"."id" = "fruit"."cake_id""#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_4() {
        use crate::{Related, Select};

        let find_fruit: Select<fruit::Entity> = cake::Entity::find_related();
        assert_eq!(
            find_fruit
                .filter(cake::Column::Id.eq(11))
                .as_query()
                .to_string(),
            [
                r#"SELECT "fruit"."id", "fruit"."name", "fruit"."cake_id" FROM "fruit""#,
                r#"INNER JOIN "cake" ON "cake"."id" = "fruit"."cake_id""#,
                r#"WHERE "cake"."id" = 11"#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_5() {
        let cake_model = cake::Model {
            id: 12,
            name: "".to_owned(),
        };

        assert_eq!(
            cake_model
                .find_related(fruit::Entity)
                .as_query()
                .to_string(),
            [
                r#"SELECT "fruit"."id", "fruit"."name", "fruit"."cake_id" FROM "fruit""#,
                r#"INNER JOIN "cake" ON "cake"."id" = "fruit"."cake_id""#,
                r#"WHERE "cake"."id" = 12"#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_6() {
        assert_eq!(
            cake::Entity::find()
                .left_join(filling::Entity)
                .as_query()
                .to_string(),
            [
                r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
                r#"LEFT JOIN "cake_filling" ON "cake"."id" = "cake_filling"."cake_id""#,
                r#"LEFT JOIN "filling" ON "cake_filling"."filling_id" = "filling"."id""#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_7() {
        use crate::{Related, Select};

        let find_filling: Select<filling::Entity> = cake::Entity::find_related();
        assert_eq!(
            find_filling.as_query().to_string(),
            [
                r#"SELECT "filling"."id", "filling"."name", "filling"."vendor_id" FROM "filling""#,
                r#"INNER JOIN "cake_filling" ON "cake_filling"."filling_id" = "filling"."id""#,
                r#"INNER JOIN "cake" ON "cake"."id" = "cake_filling"."cake_id""#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_8() {
        use crate::{Related, Select};

        let find_cake_filling_price: Select<cake_filling_price::Entity> =
            cake_filling::Entity::find_related();
        assert_eq!(
            find_cake_filling_price.as_query().to_string(),
            [
                r#"SELECT "cake_filling_price"."cake_id", "cake_filling_price"."filling_id", "cake_filling_price"."price""#,
                r#"FROM "public"."cake_filling_price""#,
                r#"INNER JOIN "cake_filling" ON"#,
                r#""cake_filling"."cake_id" = "cake_filling_price"."cake_id" AND"#,
                r#""cake_filling"."filling_id" = "cake_filling_price"."filling_id""#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_9() {
        use crate::{Related, Select};

        let find_cake_filling: Select<cake_filling::Entity> =
            cake_filling_price::Entity::find_related();
        assert_eq!(
            find_cake_filling.as_query().to_string(),
            [
                r#"SELECT "cake_filling"."cake_id", "cake_filling"."filling_id""#,
                r#"FROM "cake_filling""#,
                r#"INNER JOIN "public"."cake_filling_price" ON"#,
                r#""cake_filling_price"."cake_id" = "cake_filling"."cake_id" AND"#,
                r#""cake_filling_price"."filling_id" = "cake_filling"."filling_id""#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_10() {
        let cake_model = cake::Model {
            id: 12,
            name: "".to_owned(),
        };

        assert_eq!(
            cake_model
                .find_linked(entity_linked::CakeToFilling)
                .as_query()
                .to_string(),
            [
                r#"SELECT "filling"."id", "filling"."name", "filling"."vendor_id""#,
                r#"FROM "filling""#,
                r#"INNER JOIN "cake_filling" AS "r0" ON "r0"."filling_id" = "filling"."id""#,
                r#"INNER JOIN "cake" AS "r1" ON "r1"."id" = "r0"."cake_id""#,
                r#"WHERE "r1"."id" = 12"#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_11() {
        let cake_model = cake::Model {
            id: 18,
            name: "".to_owned(),
        };

        assert_eq!(
            cake_model
                .find_linked(entity_linked::CakeToFillingVendor)
                .as_query()
                .to_string(),
            [
                r#"SELECT "vendor"."id", "vendor"."name""#,
                r#"FROM "vendor""#,
                r#"INNER JOIN "filling" AS "r0" ON "r0"."vendor_id" = "vendor"."id""#,
                r#"INNER JOIN "cake_filling" AS "r1" ON "r1"."filling_id" = "r0"."id""#,
                r#"INNER JOIN "cake" AS "r2" ON "r2"."id" = "r1"."cake_id""#,
                r#"WHERE "r2"."id" = 18"#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_14() {
        assert_eq!(
            cake::Entity::find()
                .join(JoinType::LeftJoin, cake::Relation::TropicalFruit.def())
                .as_query()
                .to_string(),
            [
                r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
                r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id" AND "fruit"."name" LIKE '%tropical%'"#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_15() {
        let cake_model = cake::Model {
            id: 18,
            name: "".to_owned(),
        };

        assert_eq!(
            cake_model
                .find_linked(entity_linked::CheeseCakeToFillingVendor)
                .as_query()
                .to_string(),
            [
                r#"SELECT "vendor"."id", "vendor"."name""#,
                r#"FROM "vendor""#,
                r#"INNER JOIN "filling" AS "r0" ON "r0"."vendor_id" = "vendor"."id""#,
                r#"INNER JOIN "cake_filling" AS "r1" ON "r1"."filling_id" = "r0"."id""#,
                r#"INNER JOIN "cake" AS "r2" ON "r2"."id" = "r1"."cake_id" AND "r2"."name" LIKE '%cheese%'"#,
                r#"WHERE "r2"."id" = 18"#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_16() {
        let cake_model = cake::Model {
            id: 18,
            name: "".to_owned(),
        };
        assert_eq!(
            cake_model
                .find_linked(entity_linked::JoinWithoutReverse)
                .as_query()
                .to_string(),
            [
                r#"SELECT "vendor"."id", "vendor"."name""#,
                r#"FROM "vendor""#,
                r#"INNER JOIN "filling" AS "r0" ON "r0"."vendor_id" = "vendor"."id""#,
                r#"INNER JOIN "cake_filling" AS "r1" ON "r1"."filling_id" = "r0"."id""#,
                r#"INNER JOIN "cake_filling" AS "r2" ON "r2"."cake_id" = "r1"."id" AND "r2"."name" LIKE '%cheese%'"#,
                r#"WHERE "r2"."id" = 18"#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_19() {
        assert_eq!(
            cake::Entity::find()
                .join(JoinType::LeftJoin, cake::Relation::TropicalFruit.def())
                .join(
                    JoinType::LeftJoin,
                    cake_filling::Relation::Cake
                        .def()
                        .rev()
                        .on_condition(|_left, right| {
                            Expr::col((right, cake_filling::Column::CakeId))
                                .gt(10)
                                .into_condition()
                        })
                )
                .join(
                    JoinType::LeftJoin,
                    cake_filling::Relation::Filling
                        .def()
                        .on_condition(|_left, right| {
                            Expr::col((right, filling::Column::Name))
                                .like("%lemon%")
                                .into_condition()
                        })
                )
                .join(JoinType::LeftJoin, filling::Relation::Vendor.def())
                .as_query()
                .to_string(),
            [
                r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
                r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id" AND "fruit"."name" LIKE '%tropical%'"#,
                r#"LEFT JOIN "cake_filling" ON "cake"."id" = "cake_filling"."cake_id" AND "cake_filling"."cake_id" > 10"#,
                r#"LEFT JOIN "filling" ON "cake_filling"."filling_id" = "filling"."id" AND "filling"."name" LIKE '%lemon%'"#,
                r#"LEFT JOIN "vendor" ON "filling"."vendor_id" = "vendor"."id""#,
            ]
            .join(" ")
        );
    }

    // [spec:pgorm:def:entity.relation.def+12/test]    a condition attached
    // before `rev` keeps its authored roles — the closure's `left` still
    // receives the side it was written against — and a double `rev` renders
    // the same join as the unreversed def
    #[test]
    fn rev_preserves_authored_condition_roles() {
        let guarded = || {
            cake_filling::Relation::Cake
                .def()
                .on_condition(|left, _right| {
                    Expr::col((left, cake_filling::Column::CakeId))
                        .gt(10)
                        .into_condition()
                })
        };

        assert_eq!(
            cake::Entity::find()
                .join(JoinType::LeftJoin, guarded().rev())
                .as_query()
                .to_string(),
            [
                r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
                r#"LEFT JOIN "cake_filling" ON "cake"."id" = "cake_filling"."cake_id" AND "cake_filling"."cake_id" > 10"#,
            ]
            .join(" ")
        );

        assert_eq!(
            filling::Entity::find()
                .join(JoinType::LeftJoin, guarded().rev().rev())
                .as_query()
                .to_string(),
            filling::Entity::find()
                .join(JoinType::LeftJoin, guarded())
                .as_query()
                .to_string(),
        );
    }

    #[test]
    fn join_20() {
        let fruit_alias = alias("fruit_alias");

        assert_eq!(
            cake::Entity::find()
                .column_as(Expr::col((fruit_alias, fruit::Column::Name)), Name::runtime("fruit_name"))
                .join_as(
                    JoinType::LeftJoin,
                    cake::Relation::Fruit
                        .def()
                        .on_condition(|_left, right| {
                            Expr::col((right, fruit::Column::Name))
                                .like("%tropical%")
                                .into_condition()
                        }),
                    fruit_alias
                )
                .as_query()
                .to_string(),
            [
                r#"SELECT "cake"."id", "cake"."name", "fruit_alias"."name" AS "fruit_name" FROM "cake""#,
                r#"LEFT JOIN "fruit" AS "fruit_alias" ON "cake"."id" = "fruit_alias"."cake_id" AND "fruit_alias"."name" LIKE '%tropical%'"#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_21() {
        let cf_alias = alias("cake_filling_alias");

        assert_eq!(
            cake::Entity::find()
                .column_as(
                    Expr::col((cf_alias, cake_filling::Column::CakeId)),
                    Name::runtime("cake_filling_cake_id")
                )
                .join(JoinType::LeftJoin, cake::Relation::TropicalFruit.def())
                .join_as_rev(
                    JoinType::LeftJoin,
                    cake_filling::Relation::Cake
                        .def()
                        .on_condition(|left, _right| {
                            Expr::col((left, cake_filling::Column::CakeId))
                                .gt(10)
                                .into_condition()
                        }),
                    cf_alias
                )
                .as_query()
                .to_string(),
            [
                r#"SELECT "cake"."id", "cake"."name", "cake_filling_alias"."cake_id" AS "cake_filling_cake_id" FROM "cake""#,
                r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id" AND "fruit"."name" LIKE '%tropical%'"#,
                r#"LEFT JOIN "cake_filling" AS "cake_filling_alias" ON "cake_filling_alias"."cake_id" = "cake"."id" AND "cake_filling_alias"."cake_id" > 10"#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn join_22() {
        let cf_alias = alias("cake_filling_alias");

        assert_eq!(
            cake::Entity::find()
                .column_as(
                    Expr::col((cf_alias, cake_filling::Column::CakeId)),
                    Name::runtime("cake_filling_cake_id")
                )
                .join(JoinType::LeftJoin, cake::Relation::OrTropicalFruit.def())
                .join_as_rev(
                    JoinType::LeftJoin,
                    cake_filling::Relation::Cake
                        .def()
                        .condition_type(ConditionType::Any)
                        .on_condition(|left, _right| {
                            Expr::col((left, cake_filling::Column::CakeId))
                                .gt(10)
                                .into_condition()
                        }),
                    cf_alias
                )
                .as_query()
                .to_string(),
            [
                r#"SELECT "cake"."id", "cake"."name", "cake_filling_alias"."cake_id" AS "cake_filling_cake_id" FROM "cake""#,
                r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id" OR "fruit"."name" LIKE '%tropical%'"#,
                r#"LEFT JOIN "cake_filling" AS "cake_filling_alias" ON "cake_filling_alias"."cake_id" = "cake"."id" OR "cake_filling_alias"."cake_id" > 10"#,
            ]
            .join(" ")
        );
    }
}
