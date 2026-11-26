#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::{
    ActiveEnum,
    ActiveValue::Unchanged,
    entity::prelude::*,
    entity::*,
    pgorm_query::{BinOper, Expr},
};
use pretty_assertions::assert_eq;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("enum_primary_key_tests").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    insert_teas(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

pub async fn insert_teas(db: &DatabaseConnection) -> Result<(), Error> {
    use teas::*;

    let model = Model {
        id: Tea::EverydayTea,
        category: None,
        color: None,
    };

    assert_eq!(
        model,
        ActiveModel {
            id: set(Tea::EverydayTea),
            category: set(None),
            color: set(None),
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
            .one(db)
            .await?
    );

    // UNIQUE constraint failed
    assert!(
        ActiveModel {
            id: set(Tea::EverydayTea),
            category: set(Some(Category::Big)),
            color: set(Some(Color::Black)),
        }
        .insert(db)
        .await
        .is_err()
    );

    // UNIQUE constraint failed
    assert!(
        Insert::one(ActiveModel {
            id: set(Tea::EverydayTea),
            category: set(Some(Category::Big)),
            color: set(Some(Color::Black)),
        })
        .exec(db)
        .await
        .is_err()
    );

    let _ = ActiveModel {
        category: set(Some(Category::Big)),
        color: set(Some(Color::Black)),
        ..model.into_active_model()
    }
    .update(db)
    .await?;

    let model = Entity::find().one(db).await?;
    assert_eq!(
        model,
        Model {
            id: Tea::EverydayTea,
            category: Some(Category::Big),
            color: Some(Color::Black),
        }
    );
    assert_eq!(
        model,
        Entity::find()
            .filter(Column::Id.eq(Tea::EverydayTea))
            .filter(Column::Category.eq(Category::Big))
            .filter(Column::Color.eq(Color::Black))
            .one(db)
            .await?
    );
    assert_eq!(
        model,
        Entity::find()
            .filter(
                Expr::col(Column::Id)
                    .binary(BinOper::In, Expr::tuple([Tea::EverydayTea.as_enum()]))
            )
            .one(db)
            .await?
    );
    // Equivalent to the above.
    assert_eq!(
        model,
        Entity::find()
            .filter(Column::Id.is_in([Tea::EverydayTea]))
            .one(db)
            .await?
    );

    let res = model.delete(db).await?;

    assert_eq!(res, 1);
    assert_eq!(Entity::find().one_opt(db).await?, None);

    Ok(())
}
