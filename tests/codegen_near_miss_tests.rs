#![allow(unused_imports, dead_code)]

//! The entities pgorm-codegen generates from a base and a kind's table one
//! step short of a disjoint entity, compiled, and run against a live server.
//!
//! pgorm-codegen's own tests hold the writer's output to the files of
//! `codegen_near_miss`, so what is compiled and run here is what the
//! generator writes. The round trip closes here: the tables the generated
//! entities create are the ones the schema declares, statement for
//! statement, the base's composite unique keys included, so the kind's
//! foreign key onto the base's UNIQUE pair has the key it references.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo nextest run --test codegen_near_miss_tests`

pub mod common;
pub use common::{TestContext, setup::*};

use pgorm::entity::prelude::*;
use pgorm::{ConnectionTrait, Schema, set};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

mod codegen_near_miss;

use codegen_near_miss::pgorm_active_enums::ShelfKind;
use codegen_near_miss::{book, shelf};

const SCHEMA: &str = include_str!("codegen_near_miss/schema.sql");

/// The statements the generated entities write: the kind type, then each
/// table, the referenced one first.
fn statements() -> Result<Vec<String>, Error> {
    let schema = Schema::new();
    let mut statements: Vec<String> = schema
        .create_enum_from_entity(shelf::Entity)
        .iter()
        .map(ToString::to_string)
        .collect();
    statements.push(schema.create_table_from_entity(shelf::Entity)?.to_string());
    statements.push(schema.create_table_from_entity(book::Entity)?.to_string());
    Ok(statements)
}

// [spec:pgorm:sem:codegen.entity.disjoint+3/test]    the round trip: the generated near-miss entities write the schema they were read from, statement for statement, the base's two composite unique keys among them
// [spec:pgorm:sem:schema.from-entity+12/test]
#[test]
fn generated_entities_write_the_schema_read_from() -> Result<(), Error> {
    let file: Vec<&str> = SCHEMA
        .split(";\n")
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .collect();
    assert_eq!(statements()?, file);
    Ok(())
}

#[track_caller]
fn refused_by<T: std::fmt::Debug>(outcome: Result<T, Error>, state: &SqlState, constraint: &str) {
    match outcome {
        Err(Error::Postgres(error)) => {
            assert_eq!(error.code(), Some(state), "{error}");
            assert_eq!(
                error.as_db_error().and_then(|db| db.constraint()),
                Some(constraint),
                "{error}"
            );
        }
        other => panic!("expected `{constraint}` to refuse the write, got {other:?}"),
    }
}

// [spec:pgorm:sem:codegen.entity.disjoint+3/test]    live, the tables the generated entities create hold their rows to the schema's keys: a book on a shelf of its kind is written, one on a map shelf is refused by the foreign key onto the UNIQUE pair, and two unlabelled shelves outside every aisle by the key treating nulls as equal
#[pgorm_macros::test]
async fn generated_entities_create_their_tables_live() -> Result<(), Error> {
    let ctx = TestContext::new("codegen_near_miss").await;
    let db = ctx.db.get().await?;
    for statement in statements()? {
        db.execute(&statement, &[]).await?;
    }

    let shelf = |kind: ShelfKind, aisle: Option<i32>| shelf::ActiveModel {
        kind: set(kind),
        aisle: set(aisle),
        label: set(None),
        ..Default::default()
    };
    let books = shelf(ShelfKind::Book, Some(1)).insert(&db).await?;
    let maps = shelf(ShelfKind::Map, Some(2)).insert(&db).await?;
    let book = |id: i32| book::ActiveModel {
        id: set(id),
        title: set("atlas"),
        ..Default::default()
    };
    let written = book(books.id).insert(&db).await?;
    assert_eq!(written.kind, Some(ShelfKind::Book));
    refused_by(
        book(maps.id).insert(&db).await,
        &SqlState::FOREIGN_KEY_VIOLATION,
        "fk-book-id-kind",
    );

    shelf(ShelfKind::Map, None).insert(&db).await?;
    refused_by(
        shelf(ShelfKind::Map, None).insert(&db).await,
        &SqlState::UNIQUE_VIOLATION,
        "shelf_aisle_label_key",
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
