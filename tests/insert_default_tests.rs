#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::entity::prelude::*;
use pretty_assertions::assert_eq;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("insert_default_tests").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    create_insert_default(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:query.build.insert+6/test]    a batch to which no model was
// added writes nothing on any terminal, unlike an all-NotSet model, and its
// returning terminals answer with no key and no model
#[pgorm_macros::test]
async fn empty_batch_writes_no_row_anywhere() -> Result<(), Error> {
    use insert_default::*;

    let ctx = TestContext::new("insert_default_tests_empty_batch").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    let empty = || Insert::many(std::iter::empty::<ActiveModel>());

    assert_eq!(empty().exec(&db).await?, 0);
    assert_eq!(empty().exec_returning_pks(&db).await?, Vec::<i32>::new());
    assert_eq!(empty().exec_returning_models(&db).await?, []);
    assert!(Entity::find().all(&db).await?.is_empty());

    let blank = Insert::one(ActiveModel {
        ..Default::default()
    })
    .exec(&db)
    .await?;

    assert_eq!(blank, 1);
    assert_eq!(Entity::find().all(&db).await?, [Model { id: 1 }]);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:query.build.insert+6/test]    a batch of models that set no
// column inserts one default row per model rather than collapsing into one
#[pgorm_macros::test]
async fn all_not_set_models_insert_one_row_each() -> Result<(), Error> {
    use insert_default::*;

    let ctx = TestContext::new("insert_default_tests_blank_batch").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    let affected = Insert::many([
        ActiveModel {
            ..Default::default()
        },
        ActiveModel {
            ..Default::default()
        },
        ActiveModel {
            ..Default::default()
        },
    ])
    .exec(&db)
    .await?;

    assert_eq!(affected, 3);
    assert_eq!(
        Entity::find().all(&db).await?,
        [Model { id: 1 }, Model { id: 2 }, Model { id: 3 }]
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

pub async fn create_insert_default(db: &DatabaseConnection) -> Result<(), Error> {
    use insert_default::*;

    let active_model = ActiveModel {
        ..Default::default()
    };

    active_model.clone().insert(db).await?;
    active_model.clone().insert(db).await?;
    active_model.insert(db).await?;

    assert_eq!(
        Entity::find().all(db).await?,
        [Model { id: 1 }, Model { id: 2 }, Model { id: 3 }]
    );

    Ok(())
}
