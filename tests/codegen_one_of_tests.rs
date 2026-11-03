#![allow(unused_imports, dead_code)]

//! The one-of reference pgorm-codegen generates from the statements
//! `create_table_from_entity` and `Schema::create_one_of` write, compiled,
//! and run against a live server.
//!
//! pgorm-codegen's own tests hold the writer's output to the files of
//! `codegen_one_of`, so what is compiled and run here is what the generator
//! writes. The round trip closes here: the fixtures' schema, generated from,
//! is the schema the generated entities and enum write, statement for
//! statement, and on a server the generated enum reads, writes and loads
//! what the schema holds.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo nextest run --test codegen_one_of_tests`

pub mod common;
pub use common::TestContext;

use pgorm::entity::prelude::*;
use pgorm::{ConnectionTrait, OneOf, Schema, set};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

mod codegen_one_of;

use codegen_one_of::comment::{Commentable, CommentableKey, CommentableModel};
use codegen_one_of::{comment, photo, post};

const SCHEMA: &str = include_str!("codegen_one_of/schema.sql");

fn statements() -> Vec<&'static str> {
    SCHEMA
        .split(";\n")
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .collect()
}

// [spec:pgorm:def:disjoint.one-of+1/test]    the round trip: the schema the comment fixture writes, generated from, is the schema the generated entities and the generated enum's `create_one_of` write, statement for statement
#[test]
fn generated_enum_writes_the_schema_read_from() -> Result<(), Error> {
    let schema = Schema::new();
    let written = [
        schema.create_table_from_entity(post::Entity)?.to_string(),
        schema.create_table_from_entity(photo::Entity)?.to_string(),
        schema
            .create_table_from_entity(comment::Entity)?
            .to_string(),
        schema.create_one_of::<Commentable>().to_string(),
    ];
    assert_eq!(statements(), written);
    assert_eq!(
        schema.create_one_of::<Commentable>().to_string(),
        schema
            .create_one_of::<pgorm::tests_cfg::comment::Commentable>()
            .to_string()
    );
    Ok(())
}

// [spec:pgorm:def:disjoint.one-of+1/test]    on the schema it was generated from, the generated enum writes each reference, reads it back, loads each row's target, and the schema refuses a row referencing two
#[pgorm_macros::test]
async fn generated_reference_reads_writes_and_loads_live() -> Result<(), Error> {
    let ctx = TestContext::new("codegen_one_of").await;
    let db = ctx.db.get().await?;
    for statement in statements() {
        db.execute(statement, &[]).await?;
    }
    let post = post::ActiveModel {
        title: set("hello"),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    let photo = photo::ActiveModel {
        url: set("a.png"),
        ..Default::default()
    }
    .insert(&db)
    .await?;

    let mut written = Vec::new();
    for key in [
        CommentableKey::Photo(photo.id),
        CommentableKey::None,
        CommentableKey::Post(post.id),
    ] {
        let mut active = comment::ActiveModel {
            body: set("on"),
            ..Default::default()
        };
        Commentable::set(&mut active, key.clone())?;
        let comment = active.insert(&db).await?;
        assert_eq!(Commentable::key(&comment)?, key);
        written.push(comment);
    }

    let loaded = Commentable::load(&written, &db).await?;
    match loaded.as_slice() {
        [
            CommentableModel::Photo(on_photo),
            CommentableModel::None,
            CommentableModel::Post(on_post),
        ] => assert_eq!((on_photo, on_post), (&photo, &post)),
        other => panic!("loaded {other:?}"),
    }

    match db
        .execute(
            "INSERT INTO comment (body, post_id, photo_id) VALUES ('both', $1, $2)",
            &[&post.id, &photo.id],
        )
        .await
    {
        Err(Error::Postgres(error)) => {
            assert_eq!(error.code(), Some(&SqlState::CHECK_VIOLATION), "{error}")
        }
        other => panic!("expected a row referencing two targets refused, got {other:?}"),
    }

    drop(db);
    ctx.delete().await;
    Ok(())
}
