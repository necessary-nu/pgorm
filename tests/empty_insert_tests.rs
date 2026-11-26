#![allow(unused_imports, dead_code)]

use crate::common;
use crate::crud;

pub use common::{TestContext, bakery_chain::*, setup::*};
pub use pgorm::{
    DatabaseConnection, DatabasePool, EntityName, Insert, entity::*, error::Error, set, tests_cfg,
};

pub use crud::*;
use pgorm::TryInsertResult;

#[pgorm_macros::test]
async fn main() {
    let ctx = TestContext::new("bakery_chain_empty_insert_tests").await;
    create_tables(&ctx.db).await.unwrap();

    let db = ctx.db.get().await.unwrap();
    test(&db).await;
    columns_mismatch_is_refused(&db).await;

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:sem:exec.crud.try-insert+4/test]    `TryInsert::exec` reporting
// Inserted, Conflicted and Empty
// [spec:pgorm:sem:query.build.insert.empty-failsafe+5/test]    `on_empty_do_nothing`
// / `on_conflict_do_nothing` produce a `TryInsert` whose `exec` maps
// RecordNotInserted to Conflicted and an empty batch to Empty
pub async fn test(db: &DatabaseConnection) {
    let seaside_bakery = bakery::ActiveModel {
        name: set("SeaSide Bakery"),
        profit_margin: set(10.4),
        ..Default::default()
    };

    let res = Insert::one(seaside_bakery)
        .on_empty_do_nothing()
        .exec(db)
        .await;

    assert!(matches!(res, Ok(TryInsertResult::Inserted(_))));

    let double_seaside_bakery = bakery::ActiveModel {
        name: set("SeaSide Bakery"),
        profit_margin: set(10.4),
        id: set(1),
    };

    let conflict_insert = Insert::many([double_seaside_bakery])
        .on_conflict_do_nothing()
        .exec(db)
        .await;

    assert!(matches!(conflict_insert, Ok(TryInsertResult::Conflicted)));

    // [spec:pgorm:sem:query.build.insert.empty-failsafe+5] An empty batch is a
    // no-op that reports Empty before any SQL is issued.
    let empty_insert = Insert::many(std::iter::empty::<bakery::ActiveModel>())
        .on_empty_do_nothing()
        .exec(db)
        .await;

    assert!(matches!(empty_insert, Ok(TryInsertResult::Empty)));

    // A model with every column NotSet records no column either, so it reaches
    // the same empty state and leaves the database untouched.
    let blank_insert = Insert::one(bakery::ActiveModel {
        ..Default::default()
    })
    .on_empty_do_nothing()
    .exec(db)
    .await;

    assert!(matches!(blank_insert, Ok(TryInsertResult::Empty)));
    assert_eq!(Bakery::find().all(db).await.unwrap().len(), 1);
}

// [spec:pgorm:req:query.build.insert.uniform-columns+4/test]    every execution
// path of a batch, on both insert types, reports the recorded mismatch instead of sending
// SQL, so the batch leaves the database untouched
pub async fn columns_mismatch_is_refused(db: &DatabaseConnection) {
    let mismatched = || {
        Insert::many([
            bakery::ActiveModel {
                name: set("Hillside Bakery"),
                profit_margin: set(1.0),
                ..Default::default()
            },
            bakery::ActiveModel {
                id: set(9),
                name: set("Riverside Bakery"),
                profit_margin: set(2.0),
            },
        ])
    };
    let expected = "Query Error: models added to one insert do not share a column set: \
                    `id` is set in a later model but not in the first";

    let refused = [
        mismatched().exec_returning_pks(db).await.err(),
        mismatched().exec(db).await.err(),
        mismatched().exec_returning_models(db).await.err(),
        mismatched()
            .on_empty_do_nothing()
            .exec_returning_pks(db)
            .await
            .err(),
        mismatched().on_empty_do_nothing().exec(db).await.err(),
        mismatched()
            .on_empty_do_nothing()
            .exec_returning_models(db)
            .await
            .err(),
    ];

    for err in refused {
        assert_eq!(
            err.expect("a mismatched batch cannot execute").to_string(),
            expected
        );
    }

    assert_eq!(Bakery::find().all(db).await.unwrap().len(), 1);
}
