#![allow(unused_imports, dead_code)]

use crate::common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::entity::prelude::*;
use pretty_assertions::assert_eq;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("uuid_fmt_tests").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    insert_uuid_fmt(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

pub async fn insert_uuid_fmt(db: &DatabaseConnection) -> Result<(), Error> {
    let uuid = Uuid::new_v4();

    let uuid_fmt = uuid_fmt::Model {
        id: 1,
        uuid,
        uuid_braced: uuid.braced(),
        uuid_hyphenated: uuid.hyphenated(),
        uuid_simple: uuid.simple(),
        uuid_urn: uuid.urn(),
    };

    let result = uuid_fmt.clone().into_active_model().insert(db).await?;

    assert_eq!(result, uuid_fmt);

    Ok(())
}
