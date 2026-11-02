#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::entity::prelude::*;
use pretty_assertions::assert_eq;
use serde_json::json;

// [spec:pgorm:sem:macros.derive.from-query-result+4/test]    JsonValueType round-trips through the DB:
// `From<T> for Value` serialises through serde_json, `ValueType` / `TryGetableFromJson` read it
// back, and the `Option` column exercises the `Nullable` impl.
#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("json_struct_tests").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    insert_json_struct_1(&db).await?;
    insert_json_struct_2(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:exec.decode.json+1/test]    a `TryGetableFromJson` type decoded
// through the blanket `TryGetable` impl, both bare and behind an `Option`
pub async fn insert_json_struct_1(db: &DatabaseConnection) -> Result<(), Error> {
    use json_struct::*;

    let model = Model {
        id: 1,
        json: json!({
            "id": 1,
            "name": "apple",
            "price": 12.01,
            "notes": "hand picked, organic",
        }),
        json_value: KeyValue {
            id: 1,
            name: "apple".into(),
            price: 12.01,
            notes: Some("hand picked, organic".into()),
        },
        json_value_opt: Some(KeyValue {
            id: 1,
            name: "apple".into(),
            price: 12.01,
            notes: Some("hand picked, organic".into()),
        }),
    };

    let result = model.clone().into_active_model().insert(db).await?;

    assert_eq!(result, model);

    assert_eq!(
        Entity::find()
            .filter(Column::Id.eq(model.id))
            .one_opt(db)
            .await?,
        Some(model)
    );

    Ok(())
}

pub async fn insert_json_struct_2(db: &DatabaseConnection) -> Result<(), Error> {
    use json_struct::*;

    let model = Model {
        id: 2,
        json: json!({
            "id": 2,
            "name": "orange",
            "price": 10.93,
            "notes": "sweet & juicy",
        }),
        json_value: KeyValue {
            id: 1,
            name: "orange".into(),
            price: 10.93,
            notes: None,
        },
        json_value_opt: None,
    };

    let result = model.clone().into_active_model().insert(db).await?;

    assert_eq!(result, model);

    assert_eq!(
        Entity::find()
            .filter(Column::Id.eq(model.id))
            .one_opt(db)
            .await?,
        Some(model)
    );

    Ok(())
}
