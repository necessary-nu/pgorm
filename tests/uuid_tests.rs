#![allow(unused_imports, dead_code)]

use crate::common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::{entity::prelude::*, entity::*};
use pretty_assertions::assert_eq;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("bakery_chain_schema_uuid_tests").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    create_and_update_metadata(&db).await?;
    insert_metadata(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

pub async fn insert_metadata(db: &DatabaseConnection) -> Result<(), Error> {
    let metadata = metadata::Model {
        uuid: Uuid::new_v4(),
        ty: "Type".to_owned(),
        key: "markup".to_owned(),
        value: "1.18".to_owned(),
        bytes: vec![1, 2, 3],
        date: Some(Date::new(2021, 9, 27).unwrap()),
        time: Some(Time::new(11, 32, 55, 0).unwrap()),
    };

    let result = metadata.clone().into_active_model().insert(db).await?;

    assert_eq!(result, metadata);

    let found = metadata::Entity::find()
        .filter(metadata::Column::Uuid.eq(metadata.uuid))
        .one_opt(db)
        .await?;

    assert_eq!(found, Some(metadata));

    Ok(())
}

pub async fn create_and_update_metadata(db: &DatabaseConnection) -> Result<(), Error> {
    let metadata = metadata::Model {
        uuid: Uuid::new_v4(),
        ty: "Type".to_owned(),
        key: "markup".to_owned(),
        value: "1.18".to_owned(),
        bytes: vec![1, 2, 3],
        date: Some(Date::new(2021, 9, 27).unwrap()),
        time: Some(Time::new(11, 32, 55, 0).unwrap()),
    };

    let res = Insert::one(metadata.clone().into_active_model())
        .exec_returning_pk(db)
        .await?;

    assert_eq!(Metadata::find().one_opt(db).await?, Some(metadata.clone()));

    assert_eq!(res, metadata.uuid);

    let update_res = Update::one(metadata::ActiveModel {
        value: set("0.22"),
        ..metadata.clone().into_active_model()
    })?
    .filter(metadata::Column::Uuid.eq(Uuid::default()))
    .exec_returning_model(db)
    .await;

    assert_eq!(update_res, Err(Error::RecordNotFound));

    Ok(())
}
