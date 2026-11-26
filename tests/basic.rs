#![allow(unused_imports, dead_code)]

use crate::common;

pub use common::{TestContext, setup::create_table_without_asserts};
pub use pgorm::{
    ActiveValue::Unchanged, DatabaseConnection, Paginate, entity::*, error::*, pgorm_query,
    query::*, set, tests_cfg::*,
};

// DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432 cargo test --test it -- basic::
#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("basic_tests").await;
    let db = ctx.db.get().await?;

    setup_schema(&db).await?;
    crud_cake(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

async fn setup_schema(db: &DatabaseConnection) -> Result<(), Error> {
    use pgorm_query::{ColumnDef, Table};

    let stmt = Table::create(cake::Entity)
        .col(
            ColumnDef::new(cake::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(cake::Column::Id)
        .col(ColumnDef::new(cake::Column::Name).string().not_null())
        .to_owned();

    create_table_without_asserts(db, &stmt).await?;

    Ok(())
}

async fn crud_cake(db: &DatabaseConnection) -> Result<(), Error> {
    let apple = cake::ActiveModel {
        name: set("Apple Pie"),
        ..Default::default()
    };

    let apple = apple.insert(db).await?;

    assert_eq!(
        apple,
        cake::Model {
            id: 1,
            name: "Apple Pie".to_owned(),
        }
    );

    let mut apple = apple.into_active();
    apple.name = set("Lemon Tart");

    let apple = apple.update(db).await?;

    assert_eq!(
        apple,
        cake::Model {
            id: 1,
            name: "Lemon Tart".to_owned(),
        }
    );

    let count = cake::Entity::find().count(db).await?;
    assert_eq!(count, 1);

    let apple = cake::Entity::find_by_id(1).one_opt(db).await?;

    assert_eq!(
        Some(cake::Model {
            id: 1,
            name: "Lemon Tart".to_owned(),
        }),
        apple
    );

    let apple: cake::Model = apple.unwrap();
    let result = apple.delete(db).await?;
    assert_eq!(result, 1);

    let apple = cake::Entity::find_by_id(1).one_opt(db).await?;
    assert_eq!(None, apple);

    let count = cake::Entity::find().count(db).await?;
    assert_eq!(count, 0);

    Ok(())
}
