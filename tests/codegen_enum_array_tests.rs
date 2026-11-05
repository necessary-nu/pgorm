#![allow(unused_imports, dead_code)]

//! The entities pgorm-codegen generates from a table of enum-array columns,
//! against a live server.
//!
//! pgorm-codegen's own tests hold the writer to these files and compile them.
//! What only a server settles is that they are the table's columns: the table
//! the generated entity creates is the one the schema creates, every array of
//! the enum it names, the schema-qualified one in its schema and the domain
//! over one by the domain's name; and a value written through the entity is
//! read back as it was written.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::Name;
use pgorm::{ConnectionTrait, Schema, entity::prelude::*};
use pretty_assertions::assert_eq;

/// The files pgorm-codegen generates from its enum-array schema: the ones its
/// own tests hold the writer's output to, so what is run here is what the
/// generator writes.
#[path = "../pgorm-codegen/tests/sql/enum_array/pgorm_active_enums.rs"]
mod pgorm_active_enums;

#[path = "../pgorm-codegen/tests/sql/enum_array/survey.rs"]
mod survey;

use pgorm_active_enums::{Mood, Tier};

const SCHEMA: &str = include_str!("../pgorm-codegen/tests/sql/enum_array/schema.sql");

/// The schema the file's qualified enum lives in, which the file itself does
/// not create.
async fn schema(db: &impl ConnectionTrait) -> Result<(), Error> {
    db.batch_execute(&pgorm::pgorm_query::Schema::create(Name::runtime("crm")).to_string())
        .await?;
    db.batch_execute(SCHEMA).await
}

/// Each column of `survey` with its type as the server prints it and whether
/// it is `NOT NULL`.
async fn columns_of(db: &impl ConnectionTrait) -> Result<Vec<(String, String, bool)>, Error> {
    let rows = db
        .query_all(
            "SELECT attname::text, format_type(atttypid, atttypmod), attnotnull \
             FROM pg_attribute \
             WHERE attrelid = 'survey'::regclass AND attnum > 0 AND NOT attisdropped \
             ORDER BY attnum",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect())
}

fn answer(id: i32) -> survey::Model {
    survey::Model {
        id,
        moods: vec![Mood::Calm, Mood::Glad],
        past_moods: Some(vec![Mood::Sad, Mood::Sad]),
        tiers: vec![Tier::Pro, Tier::Free],
        past_tiers: Some(vec![Tier::Team]),
        log: Some(vec![Mood::Glad]),
    }
}

/// The schema, run as written, and the table the generated entity creates
/// have the same columns, each an array of the enum it names.
// [spec:pgorm:sem:codegen.entity.types+13/test]    against a live server: the entity the
// writer generates from a table of enum arrays creates that table, the schema-qualified
// enum's array in its schema
#[pgorm_macros::test]
async fn generated_enum_arrays_keep_their_types() -> Result<(), Error> {
    let ctx = TestContext::new("codegen_enum_array_tests_columns").await;
    let db = ctx.db.get().await?;

    schema(&db).await?;
    let described = columns_of(&db).await?;
    let typed: Vec<_> = described
        .iter()
        .map(|(name, typ, not_null)| (name.as_str(), typ.as_str(), *not_null))
        .collect();
    assert_eq!(
        typed,
        [
            ("id", "integer", true),
            ("moods", "mood[]", true),
            ("past_moods", "mood[]", false),
            ("tiers", "crm.tier[]", true),
            ("past_tiers", "crm.tier[]", false),
            ("log", "mood_log", false),
        ]
    );

    let create = Schema::new().create_table_from_entity(survey::Entity)?;
    db.batch_execute("DROP TABLE survey").await?;
    db.batch_execute(&create.to_string()).await?;
    assert_eq!(columns_of(&db).await?, described);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// Values written through the generated entity come back as written, empty
/// arrays and absent ones included, and an enum array is a filter's operand as
/// it is a field.
// [spec:pgorm:sem:codegen.entity.types+13/test]    against a live server: each enum-array
// field of the generated entity writes and reads back
// [spec:pgorm:def:exec.decode.array+2/test]
#[pgorm_macros::test]
async fn generated_enum_array_fields_round_trip() -> Result<(), Error> {
    let ctx = TestContext::new("codegen_enum_array_tests_values").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;

    let written = answer(1);
    assert_eq!(
        written.clone().into_active_model().insert(&db).await?,
        written
    );
    let empty = survey::Model {
        moods: Vec::new(),
        past_moods: None,
        tiers: Vec::new(),
        past_tiers: Some(Vec::new()),
        log: None,
        ..answer(2)
    };
    assert_eq!(empty.clone().into_active_model().insert(&db).await?, empty);
    let third = answer(3);
    assert_eq!(third.clone().into_active_model().insert(&db).await?, third);

    assert_eq!(survey::Entity::find_by_id(1).one(&db).await?, written);
    assert_eq!(survey::Entity::find_by_id(2).one(&db).await?, empty);
    assert_eq!(
        survey::Entity::find()
            .filter(survey::Column::Tiers.eq(vec![Tier::Pro, Tier::Free]))
            .order_by_asc(survey::Column::Id)
            .all(&db)
            .await?
            .iter()
            .map(|answer| answer.id)
            .collect::<Vec<_>>(),
        [1, 3]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
