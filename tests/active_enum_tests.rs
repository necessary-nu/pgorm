#![allow(unused_imports, dead_code)]

pub mod common;

use active_enum::Entity as ActiveEnumEntity;
pub use common::{TestContext, features::*, setup::*};
use pgorm::{
    ActiveEnum as ActiveEnumTrait,
    ActiveValue::Unchanged,
    entity::prelude::*,
    entity::*,
    pgorm_query::{BinOper, Expr},
};
use pretty_assertions::assert_eq;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("active_enum_tests").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    insert_active_enum(&db).await?;
    insert_active_enum_child(&db).await?;
    insert_active_enum_vec(&db).await?;
    find_related_active_enum(&db).await?;
    find_linked_active_enum(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

pub async fn insert_active_enum(db: &DatabaseConnection) -> Result<(), Error> {
    use active_enum::*;

    let model = Model {
        id: 1,
        category: None,
        color: None,
        tea: None,
    };

    assert_eq!(
        model,
        ActiveModel {
            category: set(None),
            color: set(None),
            tea: set(None),
            ..Default::default()
        }
        .insert(db)
        .await?
    );
    assert_eq!(model, Entity::find().one(db).await?);
    assert_eq!(
        model,
        Entity::find()
            .filter(Column::Id.is_not_null())
            .filter(Column::Category.is_null())
            .filter(Column::Color.is_null())
            .filter(Column::Tea.is_null())
            .one(db)
            .await?
    );

    let _ = ActiveModel {
        category: set(Some(Category::Big)),
        color: set(Some(Color::Black)),
        tea: set(Some(Tea::EverydayTea)),
        ..model.into_active_model()
    }
    .update(db)
    .await?;

    let model = Entity::find().one(db).await?;
    assert_eq!(
        model,
        Model {
            id: 1,
            category: Some(Category::Big),
            color: Some(Color::Black),
            tea: Some(Tea::EverydayTea),
        }
    );
    assert_eq!(
        model,
        Entity::find()
            .filter(Column::Id.eq(1))
            .filter(Column::Category.eq(Category::Big))
            .filter(Column::Color.eq(Color::Black))
            .filter(Column::Tea.eq(Tea::EverydayTea))
            .one(db)
            .await?
    );

    assert_eq!(
        model,
        Entity::find()
            .filter(
                Expr::col(Column::Tea)
                    .binary(BinOper::In, Expr::tuple([Tea::EverydayTea.as_enum()]))
            )
            .one(db)
            .await?
    );
    // Equivalent to the above.
    let select_with_tea_in =
        Entity::find().filter(Column::Tea.is_in([Tea::EverydayTea, Tea::BreakfastTea]));
    assert_eq!(
        select_with_tea_in.as_query().to_string(),
        [
            r#"SELECT "active_enum"."id","#,
            r#""active_enum"."category","#,
            r#""active_enum"."color","#,
            r#"CAST("active_enum"."tea" AS text)"#,
            r#"FROM "active_enum""#,
            r#"WHERE "active_enum"."tea" IN (CAST('EverydayTea' AS tea), CAST('BreakfastTea' AS tea))"#,
        ]
        .join(" ")
    );
    assert_eq!(model, select_with_tea_in.one(db).await?);

    assert_eq!(
        model,
        Entity::find()
            .filter(Column::Tea.is_not_null())
            .filter(
                Expr::col(Column::Tea)
                    .binary(BinOper::NotIn, Expr::tuple([Tea::BreakfastTea.as_enum()]))
            )
            .one(db)
            .await?
    );
    // Equivalent to the above.
    let select_with_tea_not_in = Entity::find()
        .filter(Column::Tea.is_not_null())
        .filter(Column::Tea.is_not_in([Tea::BreakfastTea]));

    assert_eq!(
        select_with_tea_not_in.as_query().to_string(),
        [
            r#"SELECT "active_enum"."id","#,
            r#""active_enum"."category","#,
            r#""active_enum"."color","#,
            r#"CAST("active_enum"."tea" AS text)"#,
            r#"FROM "active_enum""#,
            r#"WHERE "active_enum"."tea" IS NOT NULL"#,
            r#"AND "active_enum"."tea" NOT IN (CAST('BreakfastTea' AS tea))"#,
        ]
        .join(" ")
    );

    assert_eq!(model, select_with_tea_not_in.one(db).await?);

    // String enums are compared alphabetically.
    // 'B' < 'S', so Big is considered "smaller" than Small.
    assert_eq!(
        model,
        Entity::find()
            .filter(Column::Category.lt(Category::Small))
            .one(db)
            .await?
    );

    // Integer enums are compared by value.
    // 0 <= 1, so Black is considered "smaller or equal to" White.
    assert_eq!(
        model,
        Entity::find()
            .filter(Column::Color.lte(Color::White))
            .one(db)
            .await?
    );

    // Postgres enums are compared by their definition order
    // (see https://www.postgresql.org/docs/current/datatype-enum.html#DATATYPE-ENUM-ORDERING).
    // Tea was defined as ('EverydayTea', 'BreakfastTea'), so EverydayTea is considered "smaller" than BreakfastTea.
    assert_eq!(
        model,
        Entity::find()
            .filter(Column::Tea.lt(Tea::BreakfastTea))
            .one(db)
            .await?
    );

    let res = model.delete(db).await?;

    assert_eq!(res, 1);
    assert_eq!(Entity::find().one_opt(db).await?, None);

    Ok(())
}

pub async fn insert_active_enum_child(db: &DatabaseConnection) -> Result<(), Error> {
    use active_enum_child::*;

    active_enum::ActiveModel {
        category: set(Some(Category::Small)),
        color: set(Some(Color::White)),
        tea: set(Some(Tea::BreakfastTea)),
        ..Default::default()
    }
    .insert(db)
    .await?;

    let am = ActiveModel {
        parent_id: set(2),
        category: set(None),
        color: set(None),
        tea: set(None),
        ..Default::default()
    }
    .insert(db)
    .await?;

    let model = Entity::find().one(db).await?;
    assert_eq!(
        model,
        Model {
            id: 1,
            parent_id: 2,
            category: None,
            color: None,
            tea: None,
        }
    );
    assert_eq!(
        model,
        Entity::find()
            .filter(Column::Id.is_not_null())
            .filter(Column::Category.is_null())
            .filter(Column::Color.is_null())
            .filter(Column::Tea.is_null())
            .one(db)
            .await?
    );

    ActiveModel {
        category: set(Some(Category::Big)),
        color: set(Some(Color::Black)),
        tea: set(Some(Tea::EverydayTea)),
        ..am.into_active_model()
    }
    .update(db)
    .await?;

    let model = Entity::find().one(db).await?;
    assert_eq!(
        model,
        Model {
            id: 1,
            parent_id: 2,
            category: Some(Category::Big),
            color: Some(Color::Black),
            tea: Some(Tea::EverydayTea),
        }
    );
    assert_eq!(
        model,
        Entity::find()
            .filter(Column::Id.eq(1))
            .filter(Column::Category.eq(Category::Big))
            .filter(Column::Color.eq(Color::Black))
            .filter(Column::Tea.eq(Tea::EverydayTea))
            .one(db)
            .await?
    );

    Ok(())
}

pub async fn insert_active_enum_vec(db: &DatabaseConnection) -> Result<(), Error> {
    use categories::*;

    let model = Model {
        id: 1,
        categories: None,
    };

    assert_eq!(
        model,
        ActiveModel {
            id: set(1),
            categories: set(None),
        }
        .insert(db)
        .await?
    );
    assert_eq!(model, Entity::find().one(db).await?);
    assert_eq!(
        model,
        Entity::find()
            .filter(Column::Id.is_not_null())
            .filter(Column::Categories.is_null())
            .one(db)
            .await?
    );

    let _ = ActiveModel {
        id: set(1),
        categories: set(Some(vec![Category::Big, Category::Small])),
    }
    .update(db)
    .await?;

    let model = Entity::find().one(db).await?;
    assert_eq!(
        model,
        Model {
            id: 1,
            categories: Some(vec![Category::Big, Category::Small]),
        }
    );
    assert_eq!(
        model,
        Entity::find()
            .filter(Column::Id.eq(1))
            .filter(
                Expr::template(r#"$1 = ANY("categories")"#, vec![Category::Big])
                    .expect("template arity")
            )
            .one(db)
            .await?
    );

    let res = model.delete(db).await?;

    assert_eq!(res, 1);
    assert_eq!(Entity::find().one_opt(db).await?, None);

    Ok(())
}

pub async fn find_related_active_enum(db: &DatabaseConnection) -> Result<(), Error> {
    assert_eq!(
        active_enum::Model {
            id: 2,
            category: None,
            color: None,
            tea: None,
        }
        .find_related(ActiveEnumChild)
        .all(db)
        .await?,
        [active_enum_child::Model {
            id: 1,
            parent_id: 2,
            category: Some(Category::Big),
            color: Some(Color::Black),
            tea: Some(Tea::EverydayTea),
        }]
    );
    assert_eq!(
        ActiveEnumEntity::graph()
            .related_maybe::<active_enum_child::Entity>()
            .all_grouped(db)
            .await?,
        [(
            active_enum::Model {
                id: 2,
                category: Some(Category::Small),
                color: Some(Color::White),
                tea: Some(Tea::BreakfastTea),
            },
            vec![active_enum_child::Model {
                id: 1,
                parent_id: 2,
                category: Some(Category::Big),
                color: Some(Color::Black),
                tea: Some(Tea::EverydayTea),
            }]
        )]
    );
    assert_eq!(
        ActiveEnumEntity::graph()
            .related_maybe::<active_enum_child::Entity>()
            .all(db)
            .await?,
        [(
            active_enum::Model {
                id: 2,
                category: Some(Category::Small),
                color: Some(Color::White),
                tea: Some(Tea::BreakfastTea),
            },
            Some(active_enum_child::Model {
                id: 1,
                parent_id: 2,
                category: Some(Category::Big),
                color: Some(Color::Black),
                tea: Some(Tea::EverydayTea),
            })
        )]
    );

    assert_eq!(
        active_enum_child::Model {
            id: 1,
            parent_id: 2,
            category: None,
            color: None,
            tea: None,
        }
        .find_related(ActiveEnum)
        .all(db)
        .await?,
        [active_enum::Model {
            id: 2,
            category: Some(Category::Small),
            color: Some(Color::White),
            tea: Some(Tea::BreakfastTea),
        }]
    );
    assert_eq!(
        ActiveEnumChild::graph()
            .related_maybe::<active_enum::Entity>()
            .all_grouped(db)
            .await?,
        [(
            active_enum_child::Model {
                id: 1,
                parent_id: 2,
                category: Some(Category::Big),
                color: Some(Color::Black),
                tea: Some(Tea::EverydayTea),
            },
            vec![active_enum::Model {
                id: 2,
                category: Some(Category::Small),
                color: Some(Color::White),
                tea: Some(Tea::BreakfastTea),
            }]
        )]
    );
    assert_eq!(
        ActiveEnumChild::graph()
            .related_maybe::<active_enum::Entity>()
            .all(db)
            .await?,
        [(
            active_enum_child::Model {
                id: 1,
                parent_id: 2,
                category: Some(Category::Big),
                color: Some(Color::Black),
                tea: Some(Tea::EverydayTea),
            },
            Some(active_enum::Model {
                id: 2,
                category: Some(Category::Small),
                color: Some(Color::White),
                tea: Some(Tea::BreakfastTea),
            })
        )]
    );

    Ok(())
}

pub async fn find_linked_active_enum(db: &DatabaseConnection) -> Result<(), Error> {
    assert_eq!(
        active_enum::Model {
            id: 2,
            category: None,
            color: None,
            tea: None,
        }
        .find_linked(RelatedLink::to(active_enum_child::Entity))
        .all(db)
        .await?,
        [active_enum_child::Model {
            id: 1,
            parent_id: 2,
            category: Some(Category::Big),
            color: Some(Color::Black),
            tea: Some(Tea::EverydayTea),
        }]
    );
    assert_eq!(
        ActiveEnumEntity::graph()
            .related_maybe::<active_enum_child::Entity>()
            .all(db)
            .await?,
        [(
            active_enum::Model {
                id: 2,
                category: Some(Category::Small),
                color: Some(Color::White),
                tea: Some(Tea::BreakfastTea),
            },
            Some(active_enum_child::Model {
                id: 1,
                parent_id: 2,
                category: Some(Category::Big),
                color: Some(Color::Black),
                tea: Some(Tea::EverydayTea),
            })
        )]
    );
    assert_eq!(
        ActiveEnumEntity::graph()
            .related_maybe::<active_enum_child::Entity>()
            .all_grouped(db)
            .await?,
        [(
            active_enum::Model {
                id: 2,
                category: Some(Category::Small),
                color: Some(Color::White),
                tea: Some(Tea::BreakfastTea),
            },
            vec![active_enum_child::Model {
                id: 1,
                parent_id: 2,
                category: Some(Category::Big),
                color: Some(Color::Black),
                tea: Some(Tea::EverydayTea),
            }]
        )]
    );

    assert_eq!(
        active_enum_child::Model {
            id: 1,
            parent_id: 2,
            category: None,
            color: None,
            tea: None,
        }
        .find_linked(RelatedLink::to(active_enum::Entity))
        .all(db)
        .await?,
        [active_enum::Model {
            id: 2,
            category: Some(Category::Small),
            color: Some(Color::White),
            tea: Some(Tea::BreakfastTea),
        }]
    );
    assert_eq!(
        ActiveEnumChild::graph()
            .related_maybe::<active_enum::Entity>()
            .all(db)
            .await?,
        [(
            active_enum_child::Model {
                id: 1,
                parent_id: 2,
                category: Some(Category::Big),
                color: Some(Color::Black),
                tea: Some(Tea::EverydayTea),
            },
            Some(active_enum::Model {
                id: 2,
                category: Some(Category::Small),
                color: Some(Color::White),
                tea: Some(Tea::BreakfastTea),
            })
        )]
    );
    assert_eq!(
        ActiveEnumChild::graph()
            .related_maybe::<active_enum::Entity>()
            .all_grouped(db)
            .await?,
        [(
            active_enum_child::Model {
                id: 1,
                parent_id: 2,
                category: Some(Category::Big),
                color: Some(Color::Black),
                tea: Some(Tea::EverydayTea),
            },
            vec![active_enum::Model {
                id: 2,
                category: Some(Category::Small),
                color: Some(Color::White),
                tea: Some(Tea::BreakfastTea),
            }]
        )]
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    pub use pgorm::QueryTrait;
    pub use pretty_assertions::assert_eq;

    #[test]
    fn active_enum_find_related() {
        let active_enum_model = active_enum::Model {
            id: 1,
            category: None,
            color: None,
            tea: None,
        };
        let select = active_enum_model.find_related(ActiveEnumChild);
        assert_eq!(
            select.as_query().to_string(),
            [
                r#"SELECT "active_enum_child"."id", "active_enum_child"."parent_id", "active_enum_child"."category", "active_enum_child"."color", CAST("active_enum_child"."tea" AS text)"#,
                r#"FROM "active_enum_child""#,
                r#"INNER JOIN "active_enum" ON "active_enum"."id" = "active_enum_child"."parent_id""#,
                r#"WHERE "active_enum"."id" = 1"#,
            ]
            .join(" ")
        );

        let select = ActiveEnumEntity::graph().related_maybe::<active_enum_child::Entity>();
        assert_eq!(
            select.as_query().to_string(),
            [
                r#"SELECT "active_enum"."id" AS "s0_id", "active_enum"."category" AS "s0_category", "active_enum"."color" AS "s0_color", CAST("active_enum"."tea" AS text) AS "s0_tea","#,
                r#""active_enum_child"."id" AS "s1_id", "active_enum_child"."parent_id" AS "s1_parent_id", "active_enum_child"."category" AS "s1_category", "active_enum_child"."color" AS "s1_color", CAST("active_enum_child"."tea" AS text) AS "s1_tea""#,
                r#"FROM "active_enum""#,
                r#"LEFT JOIN "active_enum_child" ON "active_enum"."id" = "active_enum_child"."parent_id""#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn active_enum_find_linked() {
        let active_enum_model = active_enum::Model {
            id: 1,
            category: None,
            color: None,
            tea: None,
        };
        let select = active_enum_model.find_linked(RelatedLink::to(active_enum_child::Entity));
        assert_eq!(
            select.as_query().to_string(),
            [
                r#"SELECT "active_enum_child"."id", "active_enum_child"."parent_id", "active_enum_child"."category", "active_enum_child"."color", CAST("active_enum_child"."tea" AS text)"#,
                r#"FROM "active_enum_child""#,
                r#"INNER JOIN "active_enum" AS "r0" ON "r0"."id" = "active_enum_child"."parent_id""#,
                r#"WHERE "r0"."id" = 1"#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn active_enum_child_find_related() {
        let active_enum_child_model = active_enum_child::Model {
            id: 1,
            parent_id: 2,
            category: None,
            color: None,
            tea: None,
        };
        let select = active_enum_child_model.find_related(ActiveEnum);
        assert_eq!(
            select.as_query().to_string(),
            [
                r#"SELECT "active_enum"."id", "active_enum"."category", "active_enum"."color", CAST("active_enum"."tea" AS text)"#,
                r#"FROM "active_enum""#,
                r#"INNER JOIN "active_enum_child" ON "active_enum_child"."parent_id" = "active_enum"."id""#,
                r#"WHERE "active_enum_child"."id" = 1"#,
            ]
            .join(" ")
        );

        let select = ActiveEnumChild::graph().related_maybe::<active_enum::Entity>();
        assert_eq!(
            select.as_query().to_string(),
            [
                r#"SELECT "active_enum_child"."id" AS "s0_id", "active_enum_child"."parent_id" AS "s0_parent_id", "active_enum_child"."category" AS "s0_category", "active_enum_child"."color" AS "s0_color", CAST("active_enum_child"."tea" AS text) AS "s0_tea","#,
                r#""active_enum"."id" AS "s1_id", "active_enum"."category" AS "s1_category", "active_enum"."color" AS "s1_color", CAST("active_enum"."tea" AS text) AS "s1_tea""#,
                r#"FROM "active_enum_child""#,
                r#"LEFT JOIN "active_enum" ON "active_enum_child"."parent_id" = "active_enum"."id""#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn active_enum_child_find_linked() {
        let active_enum_child_model = active_enum_child::Model {
            id: 1,
            parent_id: 2,
            category: None,
            color: None,
            tea: None,
        };
        let select = active_enum_child_model.find_linked(RelatedLink::to(active_enum::Entity));
        assert_eq!(
            select.as_query().to_string(),
            [
                r#"SELECT "active_enum"."id", "active_enum"."category", "active_enum"."color", CAST("active_enum"."tea" AS text)"#,
                r#"FROM "active_enum""#,
                r#"INNER JOIN "active_enum_child" AS "r0" ON "r0"."parent_id" = "active_enum"."id""#,
                r#"WHERE "r0"."id" = 1"#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn create_enum_from() {
        use pgorm::Schema;

        let schema = Schema::new();

        assert_eq!(
            schema
                .create_enum_from_entity(active_enum::Entity)
                .iter()
                .map(|stmt| stmt.to_string())
                .collect::<Vec<_>>(),
            [r#"CREATE TYPE "tea" AS ENUM ('EverydayTea', 'BreakfastTea')"#.to_owned()]
        );

        assert_eq!(
            schema
                .create_enum_from_active_enum::<Tea>()
                .expect("Tea resolves to ColumnType::Enum")
                .to_string(),
            r#"CREATE TYPE "tea" AS ENUM ('EverydayTea', 'BreakfastTea')"#
        );
    }

    #[test]
    fn display_test() {
        assert_eq!(format!("{}", Tea::BreakfastTea), "BreakfastTea");
        assert_eq!(format!("{}", DisplayTea::BreakfastTea), "Breakfast");
        assert_eq!(format!("{}", Tea::EverydayTea), "EverydayTea");
        assert_eq!(format!("{}", DisplayTea::EverydayTea), "Everyday");
    }
}

// [spec:pgorm:sem:exec.cursor.keyset+8/test]    a cursor keyed on an enum
// column pages across boundaries — the boundary value binds under the same
// `save_as` cast every value predicate applies
// [spec:pgorm:sem:entity.traits.column.enum-cast+16/test]    enum set membership
// through `is_in` returns rows against the live database
#[pgorm_macros::test]
async fn enum_keyed_cursor_pages_across_boundaries() -> Result<(), Error> {
    use active_enum::*;

    let ctx = TestContext::new("active_enum_cursor").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    for (id, tea) in [(1, Tea::BreakfastTea), (2, Tea::EverydayTea)] {
        ActiveModel {
            id: set(id),
            tea: set(Some(tea)),
            ..Default::default()
        }
        .insert(&db)
        .await?;
    }

    // Enum comparison order is label-declaration order, where 'EverydayTea'
    // precedes 'BreakfastTea'.
    let mut cursor = Entity::find().cursor_by(Column::Tea);
    let first = cursor.first(1).all(&db).await?;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].tea, Some(Tea::EverydayTea));

    let second = cursor.after(Tea::EverydayTea).all(&db).await?;
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].tea, Some(Tea::BreakfastTea));

    let sipped = Entity::find()
        .filter(Column::Tea.is_in([Tea::BreakfastTea]))
        .all(&db)
        .await?;
    assert_eq!(sipped.len(), 1);
    assert_eq!(sipped[0].id, 1);

    drop(db);
    ctx.delete().await;
    Ok(())
}
