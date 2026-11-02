#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::{ActiveValue::Unchanged, PartialModel, entity::prelude::*, entity::*};
use pretty_assertions::assert_eq;
use serde_json::json;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("collection_tests").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    insert_collection(&db).await?;
    update_collection(&db).await?;
    select_collection(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:exec.decode.array+2/test]    `Vec<i32>`, `Vec<Uuid>` and a
// uuid format wrapper decoded element-wise from Postgres arrays
// [spec:pgorm:def:exec.cursor.binding+6/test]    `Value::Array` binding by
// recursively wrapping its elements, with a `None` array emitted as SQL NULL
pub async fn insert_collection(db: &DatabaseConnection) -> Result<(), Error> {
    use collection::*;

    let uuid = Uuid::new_v4();

    assert_eq!(
        Model {
            id: 1,
            name: "Collection 1".into(),
            integers: vec![1, 2, 3],
            integers_opt: Some(vec![1, 2, 3]),
            teas: vec![Tea::BreakfastTea],
            teas_opt: Some(vec![Tea::BreakfastTea]),
            colors: vec![Color::Black],
            colors_opt: Some(vec![Color::Black]),
            uuid: vec![uuid],
            uuid_hyphenated: vec![uuid.hyphenated()],
        }
        .into_active_model()
        .insert(db)
        .await?,
        Model {
            id: 1,
            name: "Collection 1".into(),
            integers: vec![1, 2, 3],
            integers_opt: Some(vec![1, 2, 3]),
            teas: vec![Tea::BreakfastTea],
            teas_opt: Some(vec![Tea::BreakfastTea]),
            colors: vec![Color::Black],
            colors_opt: Some(vec![Color::Black]),
            uuid: vec![uuid],
            uuid_hyphenated: vec![uuid.hyphenated()],
        }
    );

    assert_eq!(
        Model {
            id: 2,
            name: "Collection 2".into(),
            integers: vec![10, 9],
            integers_opt: None,
            teas: vec![Tea::BreakfastTea],
            teas_opt: None,
            colors: vec![Color::Black],
            colors_opt: None,
            uuid: vec![uuid],
            uuid_hyphenated: vec![uuid.hyphenated()],
        }
        .into_active_model()
        .insert(db)
        .await?,
        Model {
            id: 2,
            name: "Collection 2".into(),
            integers: vec![10, 9],
            integers_opt: None,
            teas: vec![Tea::BreakfastTea],
            teas_opt: None,
            colors: vec![Color::Black],
            colors_opt: None,
            uuid: vec![uuid],
            uuid_hyphenated: vec![uuid.hyphenated()],
        }
    );

    assert_eq!(
        Model {
            id: 3,
            name: "Collection 3".into(),
            integers: vec![],
            integers_opt: Some(vec![]),
            teas: vec![],
            teas_opt: Some(vec![]),
            colors: vec![],
            colors_opt: Some(vec![]),
            uuid: vec![uuid],
            uuid_hyphenated: vec![uuid.hyphenated()],
        }
        .into_active_model()
        .insert(db)
        .await?,
        Model {
            id: 3,
            name: "Collection 3".into(),
            integers: vec![],
            integers_opt: Some(vec![]),
            teas: vec![],
            teas_opt: Some(vec![]),
            colors: vec![],
            colors_opt: Some(vec![]),
            uuid: vec![uuid],
            uuid_hyphenated: vec![uuid.hyphenated()],
        }
    );

    Ok(())
}

pub async fn update_collection(db: &DatabaseConnection) -> Result<(), Error> {
    use collection::*;

    let uuid = Uuid::new_v4();
    let model = Entity::find_by_id(1).one(db).await?;

    ActiveModel {
        integers: set(vec![4, 5, 6]),
        integers_opt: set(Some(vec![4, 5, 6])),
        teas: set(vec![Tea::EverydayTea]),
        teas_opt: set(Some(vec![Tea::EverydayTea])),
        colors: set(vec![Color::White]),
        colors_opt: set(Some(vec![Color::White])),
        ..model.into_active_model()
    }
    .update(db)
    .await?;

    ActiveModel {
        id: Unchanged(3),
        name: set("Collection 3"),
        integers: set(vec![3, 1, 4]),
        integers_opt: set(None),
        teas: set(vec![Tea::EverydayTea]),
        teas_opt: set(None),
        colors: set(vec![Color::White]),
        colors_opt: set(None),
        uuid: set(vec![uuid]),
        uuid_hyphenated: set(vec![uuid.hyphenated()]),
    }
    .update(db)
    .await?;

    Ok(())
}

pub async fn select_collection(db: &DatabaseConnection) -> Result<(), Error> {
    use collection::*;

    #[derive(PartialModel, Debug, PartialEq)]
    #[pgorm(entity = Entity)]
    struct PartialSelectResult {
        name: String,
    }

    let result = Entity::find_by_id(1)
        .into_partial_model::<PartialSelectResult>()
        .one_opt(db)
        .await?;

    assert_eq!(
        result,
        Some(PartialSelectResult {
            name: "Collection 1".into(),
        })
    );

    Ok(())
}
