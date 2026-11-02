use crate::{EntityTrait, QuerySelect, Related, Select};
pub use pgorm_query::JoinType;

// [spec:pgorm:sem:query.build.join+5]
impl<E> Select<E>
where
    E: EntityTrait,
{
    /// Left Join with a Related Entity.
    pub fn left_join<R>(self, _: R) -> Self
    where
        R: EntityTrait,
        E: Related<R>,
    {
        self.join_join(JoinType::LeftJoin, E::to(), E::via())
    }

    /// Right Join with a Related Entity.
    pub fn right_join<R>(self, _: R) -> Self
    where
        R: EntityTrait,
        E: Related<R>,
    {
        self.join_join(JoinType::RightJoin, E::to(), E::via())
    }

    /// Inner Join with a Related Entity.
    pub fn inner_join<R>(self, _: R) -> Self
    where
        R: EntityTrait,
        E: Related<R>,
    {
        self.join_join(JoinType::InnerJoin, E::to(), E::via())
    }

    /// Join with an Entity Related to me.
    pub fn reverse_join<R>(self, _: R) -> Self
    where
        R: EntityTrait + Related<E>,
    {
        self.join_rev(JoinType::InnerJoin, R::to())
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_cfg::{cake, cake_filling, cake_filling_price, entity_linked, filling, fruit};
    use crate::{
        ColumnTrait, EntityTrait, ModelTrait, QueryFilter, QuerySelect, QueryTrait, RelationTrait,
    };
    use pgorm_query::{ConditionType, Expr, IntoCondition, JoinType, Name, alias};
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

    // [spec:pgorm:def:entity.relation.def+11/test]    a condition attached
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
