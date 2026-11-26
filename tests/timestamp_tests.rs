#![allow(unused_imports, dead_code)]

use crate::common;
pub use common::{TestContext, features::*, setup::*};
use pgorm::entity::prelude::*;
use pretty_assertions::assert_eq;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("bakery_chain_schema_timestamp_tests").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    create_applog(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

pub async fn create_applog(db: &DatabaseConnection) -> Result<(), Error> {
    let log = applog::Model {
        id: 1,
        action: "Testing".to_owned(),
        json: Json::String("HI".to_owned()),
        created_at: "2021-09-17T17:50:20+08:00".parse().unwrap(),
    };

    let res = Insert::one(log.clone().into_active_model())
        .exec_returning_pk(db)
        .await?;

    assert_eq!(log.id, res);

    let found = Applog::find().one(db).await?;
    assert_eq!(found, log);
    assert_eq!(found.created_at.to_string(), "2021-09-17T09:50:20Z");

    Ok(())
}
