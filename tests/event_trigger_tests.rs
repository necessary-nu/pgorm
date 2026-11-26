#![allow(unused_imports, dead_code)]

use crate::common;

pub use common::{
    TestContext,
    features::{
        event_trigger::{Event, Events},
        *,
    },
    setup::*,
};
use pgorm::{ActiveValue::Unchanged, entity::prelude::*, entity::*};
use pretty_assertions::assert_eq;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("event_trigger_tests").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    insert_event_trigger(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

pub async fn insert_event_trigger(db: &DatabaseConnection) -> Result<(), Error> {
    let event_trigger = event_trigger::Model {
        id: 1,
        events: Events(
            ["A", "B", "C"]
                .into_iter()
                .map(|s| Event(s.to_owned()))
                .collect(),
        ),
    };

    let result = event_trigger.clone().into_active_model().insert(db).await?;

    assert_eq!(result, event_trigger);

    let model = event_trigger::Entity::find()
        .filter(event_trigger::Column::Id.eq(event_trigger.id))
        .one_opt(db)
        .await?;

    assert_eq!(model, Some(event_trigger));

    Ok(())
}
