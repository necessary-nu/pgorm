#![allow(unused_imports, dead_code)]

use crate::common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::entity::prelude::*;
use pretty_assertions::assert_eq;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("self_join_tests").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    create_metadata(&db).await?;
    drop(db);

    ctx.delete().await;

    Ok(())
}

pub async fn create_metadata(db: &DatabaseConnection) -> Result<(), Error> {
    let model = self_join::Model {
        uuid: Uuid::new_v4(),
        uuid_ref: None,
        time: Some(Time::new(1, 0, 0, 0).unwrap()),
    };

    model.clone().into_active_model().insert(db).await?;

    let linked_model = self_join::Model {
        uuid: Uuid::new_v4(),
        uuid_ref: Some(model.clone().uuid),
        time: Some(Time::new(2, 0, 0, 0).unwrap()),
    };

    linked_model.clone().into_active_model().insert(db).await?;

    let not_linked_model = self_join::Model {
        uuid: Uuid::new_v4(),
        uuid_ref: None,
        time: Some(Time::new(3, 0, 0, 0).unwrap()),
    };

    not_linked_model
        .clone()
        .into_active_model()
        .insert(db)
        .await?;

    assert_eq!(
        model
            .find_linked(RelatedLink::to(self_join::Entity))
            .all(db)
            .await?,
        []
    );

    assert_eq!(
        linked_model
            .find_linked(RelatedLink::to(self_join::Entity))
            .all(db)
            .await?,
        std::slice::from_ref(&model)
    );

    assert_eq!(
        not_linked_model
            .find_linked(RelatedLink::to(self_join::Entity))
            .all(db)
            .await?,
        []
    );

    assert_eq!(
        self_join::Entity::graph()
            .join_maybe_as::<self_join::Entity>(
                self_join::Relation::SelfReferencing.def(),
                alias("r0")
            )
            .order_by_asc(self_join::Column::Time)
            .all(db)
            .await?,
        [
            (model.clone(), None),
            (linked_model, Some(model)),
            (not_linked_model, None),
        ]
    );

    Ok(())
}

// [spec:pgorm:req:entity.relation.linked+5/test]    a self-relation is the case
// `RelatedLink` exists for: the link form aliases the joined table, so the
// entity's own table can be joined a second time without being named twice
#[test]
fn self_related_link_aliases_the_second_copy() {
    use pgorm::{Linked, Statement};

    assert_eq!(
        RelatedLink::<self_join::Entity, self_join::Entity>::new()
            .find_linked()
            .as_query()
            .to_string(),
        [
            r#"SELECT "self_join"."uuid", "self_join"."uuid_ref", "self_join"."time""#,
            r#"FROM "self_join""#,
            r#"INNER JOIN "self_join" AS "r0" ON "r0"."uuid_ref" = "self_join"."uuid""#,
        ]
        .join(" ")
    );
}
