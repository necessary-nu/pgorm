#![allow(unused_imports, dead_code)]

//! The entity pgorm-codegen generates from a dump that creates its own
//! schema, against a live server.
//!
//! pgorm-codegen's own tests hold the writer to these files and compile them.
//! What only a server settles is that the file runs as written, its `CREATE
//! SCHEMA` included; that the table the generated entity creates is the one
//! the file creates, in the schema the file creates; and that a value written
//! through the entity lands in that schema's table, not in a same-named one
//! `search_path` reaches first, and is read back as written.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::{ConnectionTrait, Schema, entity::prelude::*};
use pretty_assertions::assert_eq;

/// The files pgorm-codegen generates from its created-schema fixture: the
/// ones its own tests hold the writer's output to, so what is run here is
/// what the generator writes.
#[path = "../pgorm-codegen/tests/sql/created_schema/pgorm_active_enums.rs"]
mod pgorm_active_enums;

#[path = "../pgorm-codegen/tests/sql/created_schema/deal.rs"]
mod deal;

use pgorm_active_enums::Stage;

const SCHEMA: &str = include_str!("../pgorm-codegen/tests/sql/created_schema/schema.sql");

/// Each column of `sales.deal` with its type as the server prints it and
/// whether it is `NOT NULL`.
async fn columns_of(db: &impl ConnectionTrait) -> Result<Vec<(String, String, bool)>, Error> {
    let rows = db
        .query_all(
            "SELECT attname::text, format_type(atttypid, atttypmod), attnotnull \
             FROM pg_attribute \
             WHERE attrelid = 'sales.deal'::regclass AND attnum > 0 AND NOT attisdropped \
             ORDER BY attnum",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect())
}

fn won(id: i32) -> deal::Model {
    deal::Model {
        id,
        title: "Renewal".to_owned(),
        stage: Stage::Won,
        history: Some(vec![Stage::Lead, Stage::Won]),
    }
}

/// The file, run as written, creates its schema and the table in it, and
/// the table the generated entity creates there is the same table.
// [spec:pgorm:sem:codegen.ddl.objects+10/test]    against a live server: the file that
// creates its schema runs as written, and the entity generated from it creates its table
// in that schema
#[pgorm_macros::test]
async fn generated_entities_create_the_schemas_table() -> Result<(), Error> {
    let ctx = TestContext::new("codegen_schema_tests_columns").await;
    let db = ctx.db.get().await?;

    db.batch_execute(SCHEMA).await?;
    let described = columns_of(&db).await?;
    let typed: Vec<_> = described
        .iter()
        .map(|(name, typ, not_null)| (name.as_str(), typ.as_str(), *not_null))
        .collect();
    assert_eq!(
        typed,
        [
            ("id", "integer", true),
            ("title", "text", true),
            ("stage", "sales.stage", true),
            ("history", "sales.stage[]", false),
        ]
    );

    let create = Schema::new().create_table_from_entity(deal::Entity)?;
    db.batch_execute("DROP TABLE sales.deal").await?;
    db.batch_execute(&create.to_string()).await?;
    assert_eq!(columns_of(&db).await?, described);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A row written through the generated entity is the schema's, a same-named
/// table on `search_path` left empty, and reads back as written.
// [spec:pgorm:sem:codegen.ddl.objects+10/test]    against a live server: the generated
// entity writes to and reads from the table in the created schema
// [spec:pgorm:sem:codegen.entity.context+4/test]
#[pgorm_macros::test]
async fn generated_entities_write_the_schemas_table() -> Result<(), Error> {
    let ctx = TestContext::new("codegen_schema_tests_values").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SCHEMA).await?;
    db.batch_execute(
        "CREATE TABLE public.deal (id integer PRIMARY KEY, title text NOT NULL, \
         stage text NOT NULL, history text[])",
    )
    .await?;

    let written = won(1);
    assert_eq!(
        written.clone().into_active_model().insert(&db).await?,
        written
    );
    let pilot = deal::Model {
        id: 2,
        title: "Pilot".to_owned(),
        stage: Stage::Lead,
        history: None,
    };
    assert_eq!(pilot.clone().into_active_model().insert(&db).await?, pilot);

    assert_eq!(
        deal::Entity::find_by_id(2).one(&db).await?.stage,
        Stage::Lead
    );
    assert_eq!(
        deal::Entity::find()
            .filter(deal::Column::Stage.eq(Stage::Won))
            .all(&db)
            .await?
            .into_iter()
            .map(|deal| deal.id)
            .collect::<Vec<_>>(),
        [1]
    );
    assert_eq!(deal::Entity::find_by_id(1).one(&db).await?, written);
    let elsewhere: i64 = db
        .query_one("SELECT count(*) FROM public.deal", &[])
        .await?
        .get(0);
    assert_eq!(elsewhere, 0);

    drop(db);
    ctx.delete().await;
    Ok(())
}
