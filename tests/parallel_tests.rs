#![allow(unused_imports, dead_code)]

use crate::common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::entity::prelude::*;
use pretty_assertions::assert_eq;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("features_parallel_tests").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    crud_in_parallel(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

pub async fn crud_in_parallel(db: &DatabaseConnection) -> Result<(), Error> {
    let metadata = [
        metadata::Model {
            uuid: Uuid::new_v4(),
            ty: "Type".to_owned(),
            key: "markup".to_owned(),
            value: "1.18".to_owned(),
            bytes: vec![1, 2, 3],
            date: Some(Date::new(2021, 9, 27).unwrap()),
            time: Some(Time::new(11, 32, 55, 0).unwrap()),
        },
        metadata::Model {
            uuid: Uuid::new_v4(),
            ty: "Type".to_owned(),
            key: "exchange_rate".to_owned(),
            value: "0.78".to_owned(),
            bytes: vec![1, 2, 3],
            date: Some(Date::new(2021, 9, 27).unwrap()),
            time: Some(Time::new(11, 32, 55, 0).unwrap()),
        },
        metadata::Model {
            uuid: Uuid::new_v4(),
            ty: "Type".to_owned(),
            key: "service_charge".to_owned(),
            value: "1.1".to_owned(),
            bytes: vec![1, 2, 3],
            date: None,
            time: None,
        },
    ];

    let _insert_res = futures::try_join!(
        metadata[0].clone().into_active_model().insert(db),
        metadata[1].clone().into_active_model().insert(db),
        metadata[2].clone().into_active_model().insert(db),
    )?;

    let find_res = futures::try_join!(
        Metadata::find_by_id(metadata[0].uuid).one_opt(db),
        Metadata::find_by_id(metadata[1].uuid).one_opt(db),
        Metadata::find_by_id(metadata[2].uuid).one_opt(db),
    )?;

    assert_eq!(
        metadata,
        [
            find_res.0.clone().unwrap(),
            find_res.1.clone().unwrap(),
            find_res.2.clone().unwrap(),
        ]
    );

    let mut active_models = (
        find_res.0.unwrap().into_active_model(),
        find_res.1.unwrap().into_active_model(),
        find_res.2.unwrap().into_active_model(),
    );

    active_models.0.bytes = set(vec![0]);
    active_models.1.bytes = set(vec![1]);
    active_models.2.bytes = set(vec![2]);

    let _update_res = futures::try_join!(
        active_models.0.clone().update(db),
        active_models.1.clone().update(db),
        active_models.2.clone().update(db),
    )?;

    let find_res = futures::try_join!(
        Metadata::find_by_id(metadata[0].uuid).one_opt(db),
        Metadata::find_by_id(metadata[1].uuid).one_opt(db),
        Metadata::find_by_id(metadata[2].uuid).one_opt(db),
    )?;

    assert_eq!(
        [
            active_models.0.bytes.clone().unwrap(),
            active_models.1.bytes.clone().unwrap(),
            active_models.2.bytes.clone().unwrap(),
        ],
        [
            find_res.0.clone().unwrap().bytes,
            find_res.1.clone().unwrap().bytes,
            find_res.2.clone().unwrap().bytes,
        ]
    );

    let _delete_res = futures::try_join!(
        active_models.0.delete(db),
        active_models.1.delete(db),
        active_models.2.delete(db),
    )?;

    assert_eq!(Metadata::find().all(db).await?, []);

    Ok(())
}
