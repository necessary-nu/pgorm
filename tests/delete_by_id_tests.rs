#![allow(unused_imports, dead_code)]

use crate::common;
pub use common::{TestContext, features::*, setup::*};
use pgorm::entity::prelude::*;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("delete_by_id_tests").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    create_and_delete_applog(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:exec.crud.delete+1/test]    `DeleteMany::exec` reports rows
// affected, and deleting zero rows is `Ok(0)` rather than an error
pub async fn create_and_delete_applog(db: &DatabaseConnection) -> Result<(), Error> {
    let log1 = applog::Model {
        id: 1,
        action: "Testing".to_owned(),
        json: Json::String("HI".to_owned()),
        created_at: "2021-09-17T17:50:20+08:00".parse().unwrap(),
    };

    Insert::one(log1.clone().into_active_model())
        .exec(db)
        .await?;

    let log2 = applog::Model {
        id: 2,
        action: "Tests".to_owned(),
        json: Json::String("HELLO".to_owned()),
        created_at: "2022-09-17T17:50:20+08:00".parse().unwrap(),
    };

    Insert::one(log2.clone().into_active_model())
        .exec(db)
        .await?;

    let delete_res = Applog::delete_by_id(2).exec(db).await?;
    assert_eq!(delete_res, 1);

    let find_res = Applog::find_by_id(1).all(db).await?;
    assert_eq!(find_res, [log1]);

    let find_res = Applog::find_by_id(2).all(db).await?;
    assert_eq!(find_res, []);

    let delete_res = Applog::delete_by_id(3).exec(db).await?;
    assert_eq!(delete_res, 0);

    Ok(())
}
