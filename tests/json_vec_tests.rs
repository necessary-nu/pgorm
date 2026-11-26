#![allow(unused_imports, dead_code)]

use crate::common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::entity::prelude::*;
use pretty_assertions::assert_eq;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("json_vec_tests").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    insert_json_vec(&db).await?;
    insert_json_string_vec_derive(&db).await?;
    insert_json_struct_vec_derive(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

pub async fn insert_json_vec(db: &DatabaseConnection) -> Result<(), Error> {
    let json_vec = json_vec::Model {
        id: 1,
        str_vec: Some(json_vec::StringVec(vec![
            "1".to_string(),
            "2".to_string(),
            "3".to_string(),
        ])),
    };

    let result = json_vec.clone().into_active_model().insert(db).await?;

    assert_eq!(result, json_vec);

    let model = json_vec::Entity::find()
        .filter(json_vec::Column::Id.eq(json_vec.id))
        .one_opt(db)
        .await?;

    assert_eq!(model, Some(json_vec));

    Ok(())
}

pub async fn insert_json_string_vec_derive(db: &DatabaseConnection) -> Result<(), Error> {
    let json_vec = json_vec_derive::json_string_vec::Model {
        id: 1,
        str_vec: Some(json_vec_derive::json_string_vec::StringVec(vec![
            "4".to_string(),
            "5".to_string(),
            "6".to_string(),
        ])),
    };

    let result = json_vec_derive::json_string_vec::ActiveModel {
        id: NotSet,
        ..json_vec.clone().into_active_model()
    }
    .insert(db)
    .await?;

    assert_eq!(result, json_vec);

    let model = json_vec_derive::json_string_vec::Entity::find()
        .filter(json_vec_derive::json_string_vec::Column::Id.eq(json_vec.id))
        .one_opt(db)
        .await?;

    assert_eq!(model, Some(json_vec));

    Ok(())
}

// [spec:pgorm:def:exec.decode.json+2/test]    `Vec<T>` of a `TryGetableFromJson`
// type decoded through `TryGetableArray` / `from_json_vec`
pub async fn insert_json_struct_vec_derive(db: &DatabaseConnection) -> Result<(), Error> {
    let json_vec = json_vec_derive::json_struct_vec::Model {
        id: 2,
        struct_vec: vec![
            json_vec_derive::json_struct_vec::JsonColumn {
                value: "4".to_string(),
            },
            json_vec_derive::json_struct_vec::JsonColumn {
                value: "5".to_string(),
            },
            json_vec_derive::json_struct_vec::JsonColumn {
                value: "6".to_string(),
            },
        ],
    };

    let _result = json_vec.clone().into_active_model().insert(db).await?;

    let model = json_vec_derive::json_struct_vec::Entity::find()
        .filter(json_vec_derive::json_struct_vec::Column::Id.eq(json_vec.id))
        .one_opt(db)
        .await?;

    assert_eq!(model, Some(json_vec));

    Ok(())
}
