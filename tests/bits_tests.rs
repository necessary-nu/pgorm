#![allow(unused_imports, dead_code)]

pub mod common;

use common::features::*;
use pgorm::{ActiveValue::Unchanged, entity::prelude::*, entity::*};
use pretty_assertions::assert_eq;

// [spec:pgorm:req:sql.render.cast-param-type+4/test]
#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = common::TestContext::new("bits_tests_bindtypes").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    create_and_update(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

pub async fn create_and_update(db: &DatabaseConnection) -> Result<(), Error> {
    let bits = bits::Model {
        id: 1,
        bit0: 0,
        bit1: 1,
        bit8: 8,
        bit16: 16,
        bit32: 32,
        bit64: 64,
    };

    let res = bits.clone().into_active_model().insert(db).await?;

    let model = Bits::find().one_opt(db).await?;
    assert_eq!(model, Some(res));
    assert_eq!(model, Some(bits.clone()));

    let res = bits::ActiveModel {
        bit32: set(320),
        bit64: set(640),
        ..bits.clone().into_active_model()
    }
    .update(db)
    .await?;

    let model = Bits::find().one_opt(db).await?;
    assert_eq!(model, Some(res));
    assert_eq!(
        model,
        Some(bits::Model {
            id: 1,
            bit0: 0,
            bit1: 1,
            bit8: 8,
            bit16: 16,
            bit32: 320,
            bit64: 640,
        })
    );

    Ok(())
}

// [spec:pgorm:req:sql.ddl.column-types+8/test]    each bit spelling creates the
// type `format_type` prints back, and the unlengthed one holds a bit string of
// any length where SQL's bare `bit` holds one bit
#[pgorm_macros::test]
async fn bit_spellings_create_the_types_they_name() -> Result<(), Error> {
    use pgorm::pgorm_query::{ColumnDef, ColumnType, Name, Table};
    use std::sync::Arc;
    use tokio_postgres::error::SqlState;

    let n = Name::runtime;

    let ctx = common::TestContext::new("bits_tests_spellings").await;
    let db = ctx.db.get().await?;
    let create = Table::create(n("bit_spellings"))
        .col(ColumnDef::new(n("free")).bit(None))
        .col(ColumnDef::new(n("one")).bit(Some(1)))
        .col(ColumnDef::new(n("eight")).bit(Some(8)))
        .col(ColumnDef::new(n("vary")).varbit(8))
        .col(ColumnDef::new_with_type(
            n("frees"),
            ColumnType::Array(Arc::new(ColumnType::Bit(None))),
        ))
        .to_string();
    db.execute(&create, &[]).await?;

    let rows = db
        .query_all(
            "SELECT attname::text, format_type(atttypid, atttypmod) FROM pg_attribute \
             WHERE attrelid = 'bit_spellings'::regclass AND attnum > 0 ORDER BY attnum",
            &[],
        )
        .await?;
    let types: Vec<(String, String)> = rows.iter().map(|row| (row.get(0), row.get(1))).collect();
    let expected = [
        ("free", r#""bit""#),
        ("one", "bit(1)"),
        ("eight", "bit(8)"),
        ("vary", "bit varying(8)"),
        ("frees", r#""bit"[]"#),
    ];
    assert_eq!(
        types,
        expected.map(|(name, typ)| (name.to_owned(), typ.to_owned()))
    );

    db.execute(
        "INSERT INTO bit_spellings (free, frees) VALUES (B'10101', ARRAY[B'1', B'110'])",
        &[],
    )
    .await?;
    db.execute("INSERT INTO bit_spellings (free) VALUES (B'1')", &[])
        .await?;
    let lengths: Vec<i32> = db
        .query_all("SELECT length(free) FROM bit_spellings ORDER BY 1", &[])
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(lengths, [1, 5]);
    match db
        .execute("INSERT INTO bit_spellings (one) VALUES (B'10')", &[])
        .await
    {
        Err(Error::Postgres(e)) => {
            assert_eq!(e.code(), Some(&SqlState::STRING_DATA_LENGTH_MISMATCH))
        }
        other => panic!("expected a length mismatch, got {other:?}"),
    }

    drop(db);
    ctx.delete().await;
    Ok(())
}
