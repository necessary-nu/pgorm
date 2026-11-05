#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::{ActiveValue::Unchanged, entity::prelude::*, entity::*};
use pretty_assertions::assert_eq;
use serde_json::json;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("features_schema_string_primary_key_tests").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    create_and_update_repository(&db).await?;
    insert_and_delete_repository(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:exec.crud.insert+6/test]    an empty `RETURNING` fails with
// RecordNotInserted, for a client-supplied key exactly as for a generated one
pub async fn insert_and_delete_repository(db: &DatabaseConnection) -> Result<(), Error> {
    let repository = repository::Model {
        id: "unique-id-001".to_owned(),
        owner: "GC".to_owned(),
        name: "G.C.".to_owned(),
        description: None,
    }
    .into_active_model();

    let result = repository.clone().insert(db).await?;

    assert_eq!(
        result,
        repository::Model {
            id: "unique-id-001".to_owned(),
            owner: "GC".to_owned(),
            name: "G.C.".to_owned(),
            description: None,
        }
    );

    {
        use pgorm::pgorm_query::OnConflict;

        let err = Insert::one(repository.clone())
            .on_conflict(OnConflict::do_nothing())
            .exec_returning_pk(db)
            .await;

        assert_eq!(err.err(), Some(Error::RecordNotInserted));

        // [spec:pgorm:req:exec.crud.exec-vocabulary+2/test]    `exec` asks for no
        // key, so a skipped row is a count of zero rather than an error: the
        // terminal's name is the whole difference.
        let skipped = Insert::one(repository)
            .on_conflict(OnConflict::do_nothing())
            .exec(db)
            .await?;
        assert_eq!(skipped, 0);
    }

    result.delete(db).await?;

    assert_eq!(
        edit_log::Entity::find().all(db).await?,
        [
            edit_log::Model {
                id: 1,
                action: "before_save".into(),
                values: json!({
                    "description": null,
                    "id": "unique-id-001",
                    "name": "G.C.",
                    "owner": "GC",
                }),
            },
            edit_log::Model {
                id: 2,
                action: "after_save".into(),
                values: json!({
                    "description": null,
                    "id": "unique-id-001",
                    "name": "G.C.",
                    "owner": "GC",
                }),
            },
            edit_log::Model {
                id: 3,
                action: "before_delete".into(),
                values: json!({
                    "description": null,
                    "id": "unique-id-001",
                    "name": "G.C.",
                    "owner": "GC",
                }),
            },
            edit_log::Model {
                id: 4,
                action: "after_delete".into(),
                values: json!({
                    "description": null,
                    "id": "unique-id-001",
                    "name": "G.C.",
                    "owner": "GC",
                }),
            },
        ]
    );

    Ok(())
}

// [spec:pgorm:sem:exec.crud.insert+6/test]    a client-supplied primary key is
// answered from the RETURNING row like any other, not echoed back from the model
// [spec:pgorm:sem:query.build.insert+6/test]    which is why `Insert::add` keeps
// no primary-key value tuple to echo
// [spec:pgorm:sem:exec.crud.update+7/test]    `UpdateOne::exec_returning_model`
// returns the model
// built from the full-column RETURNING, including a column set back to NULL
pub async fn create_and_update_repository(db: &DatabaseConnection) -> Result<(), Error> {
    let repository = repository::Model {
        id: "unique-id-002".to_owned(),
        owner: "GC".to_owned(),
        name: "G.C.".to_owned(),
        description: None,
    };

    let res = Insert::one(repository.clone().into_active_model())
        .exec_returning_pk(db)
        .await?;

    assert_eq!(
        Repository::find().one_opt(db).await?,
        Some(repository.clone())
    );

    assert_eq!(res, repository.id);

    let updated_active_model = repository::ActiveModel {
        description: "description...".into(),
        ..repository.clone().into_active_model()
    };

    let update_res = Update::one(updated_active_model.clone())?
        .filter(repository::Column::Id.eq("not-exists-id".to_owned()))
        .exec_returning_model(db)
        .await;

    // [spec:pgorm:sem:exec.crud.update+7] UpdateOne decodes through `one`, so a
    // filter matching zero rows surfaces RecordNotFound.
    assert_eq!(update_res, Err(Error::RecordNotFound));

    let update_res = Update::one(updated_active_model)?
        .filter(repository::Column::Id.eq("unique-id-002".to_owned()))
        .exec_returning_model(db)
        .await?;

    assert_eq!(
        update_res,
        repository::Model {
            id: "unique-id-002".to_owned(),
            owner: "GC".to_owned(),
            name: "G.C.".to_owned(),
            description: Some("description...".to_owned()),
        }
    );

    let updated_active_model = repository::ActiveModel {
        description: set(None),
        ..repository.clone().into_active_model()
    };

    let update_res = Update::one(updated_active_model.clone())?
        .filter(repository::Column::Id.eq("unique-id-002".to_owned()))
        .exec_returning_model(db)
        .await?;

    assert_eq!(
        update_res,
        repository::Model {
            id: "unique-id-002".to_owned(),
            owner: "GC".to_owned(),
            name: "G.C.".to_owned(),
            description: None,
        }
    );

    Ok(())
}
