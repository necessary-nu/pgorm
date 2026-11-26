#![allow(unused_imports, dead_code)]

use crate::common;
use crate::crud;

pub use common::{TestContext, bakery_chain::*, setup::*};
pub use crud::*;
use pgorm::DatabaseConnection;

// Run the test locally:
// DATABASE_URL="postgres://postgres:postgres@localhost" cargo test --test it -- crud_tests::
#[pgorm_macros::test]
async fn main() {
    let ctx = TestContext::new("bakery_chain_schema_crud_tests").await;
    create_tables(&ctx.db).await.unwrap();

    let db = ctx.db.get().await.unwrap();
    create_entities(&db).await;
    drop(db);

    ctx.delete().await;
}

pub async fn create_entities(db: &DatabaseConnection) {
    test_create_bakery(db).await;
    test_create_baker(db).await;
    test_create_customer(db).await;
    test_create_cake(db).await;
    test_create_lineitem(db).await;
    test_create_order(db).await;

    test_update_cake(db).await;
    test_update_bakery(db).await;
    test_update_deleted_customer(db).await;

    test_delete_cake(db).await;
    test_cake_error(db).await;
    test_delete_bakery(db).await;
}
