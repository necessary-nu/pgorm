#![allow(unused_imports, dead_code)]

//! The entity pgorm-codegen generates from a table of every kind of column
//! type it reads, compiled, and run against a live server.
//!
//! pgorm-codegen's own tests hold the writer's output to the files of
//! `codegen_column_types`, so what is compiled and run here is what the
//! generator writes. The table the generated entity creates is the one the
//! schema creates, typmod for typmod, and a value written through the entity
//! is held to the column's length, precision and scale — padded, rounded or
//! refused as the column makes it.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::Range;
use pgorm::{ConnectionTrait, Schema, entity::prelude::*};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

mod codegen_column_types;

use codegen_column_types::pgorm_active_enums::Grade;
use codegen_column_types::pgorm_range_types::Textrange;
use codegen_column_types::specimen;

const SCHEMA: &str = include_str!("codegen_column_types/schema.sql");

/// Each column of `specimen` with its type as the server prints it, typmod
/// included, and whether it is `NOT NULL`.
async fn columns_of(db: &impl ConnectionTrait) -> Result<Vec<(String, String, bool)>, Error> {
    let rows = db
        .query_all(
            "SELECT attname::text, format_type(atttypid, atttypmod), attnotnull \
             FROM pg_attribute \
             WHERE attrelid = 'specimen'::regclass AND attnum > 0 AND NOT attisdropped \
             ORDER BY attnum",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect())
}

/// The schema's types, and the table the generated entity creates in place
/// of the schema's own.
async fn entity_table(db: &impl ConnectionTrait) -> Result<(), Error> {
    db.batch_execute(SCHEMA).await?;
    db.batch_execute("DROP TABLE specimen").await?;
    db.batch_execute(
        &Schema::new()
            .create_table_from_entity(specimen::Entity)?
            .to_string(),
    )
    .await
}

#[track_caller]
fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

fn date(year: i16, month: i8, day: i8) -> Date {
    jiff::civil::date(year, month, day)
}

fn specimen(id: i32) -> specimen::Model {
    specimen::Model {
        id,
        code: "ABC".to_owned(),
        title: Some("Specimen".to_owned()),
        note: "unbounded".to_owned(),
        initials: "A".to_owned(),
        flag: "Y".to_owned(),
        body: "text".to_owned(),
        codes: vec!["X".to_owned(), "YZ".to_owned()],
        marks: Some(vec!["ab".to_owned()]),
        notes: vec!["n".to_owned()],
        price: Decimal::new(1005, 3),
        whole: Decimal::new(25, 1),
        amount: Decimal::new(12345, 4),
        prices: vec![Decimal::new(10005, 4)],
        cost: None,
        small: 1,
        big: 2,
        ratio: 0.5,
        weight: 0.25,
        doc: serde_json::json!({"a": 1}),
        data: serde_json::json!([1, 2]),
        blob: vec![1, 2, 3],
        ok: true,
        uid: Uuid::from_u128(1),
        day: date(2026, 11, 2),
        clock: jiff::civil::time(12, 30, 0, 0),
        seen: date(2026, 11, 2).at(12, 30, 0, 0),
        stamped: DateTimeWithTimeZone::UNIX_EPOCH,
        wait: Interval::new(0, 1, 0),
        months: Interval::new(14, 0, 0),
        span: Range::from(1..5),
        days: [Range::from(date(2026, 1, 1)..date(2026, 2, 1))]
            .into_iter()
            .collect(),
        level: Grade::High,
        letters: Textrange(Range::from("a".to_owned().."m".to_owned())),
    }
}

/// The row of `model`, its `money` column left out: a `Decimal` field is
/// bound and decoded as `numeric`, which a `money` column is not.
fn without_cost(model: specimen::Model) -> specimen::ActiveModel {
    let mut active = model.into_active_model();
    active.cost = NotSet;
    active
}

/// The compiled entity defines each column with the type modifiers the schema
/// writes, and its range over `text` with that subtype, which neither
/// `String` nor the newtype's `Range<String>` would infer.
// [spec:pgorm:sem:codegen.entity.compact.attrs+10/test]    the compiled compact entity
// renders each column's length, precision and scale into the table it creates
#[test]
fn the_entity_table_renders_each_type_modifier() {
    let created = Schema::new()
        .create_table_from_entity(specimen::Entity)
        .expect("the entity's table is generated")
        .to_string();
    for column in [
        r#""code" varchar(3) NOT NULL"#,
        r#""title" varchar(255),"#,
        r#""note" varchar NOT NULL"#,
        r#""initials" char(2) NOT NULL"#,
        r#""flag" char(1) NOT NULL"#,
        r#""body" text NOT NULL"#,
        r#""codes" varchar(3)[] NOT NULL"#,
        r#""marks" char(4)[],"#,
        r#""notes" varchar[] NOT NULL"#,
        r#""price" decimal(10, 2) NOT NULL"#,
        r#""whole" decimal(6, 0) NOT NULL"#,
        r#""amount" decimal NOT NULL"#,
        r#""prices" decimal(8, 3)[] NOT NULL"#,
        r#""cost" money,"#,
        r#""data" jsonb NOT NULL"#,
        r#""months" interval YEAR TO MONTH NOT NULL"#,
    ] {
        assert!(created.contains(column), "{column}: {created}");
    }
    assert_eq!(
        specimen::Column::Letters.def().get_column_type(),
        &ColumnType::CreatedRange {
            name: Name::runtime("textrange"),
            schema: None,
            subtype: Arc::new(ColumnType::Text),
        }
    );
}

/// The schema, run as written, and the table its generated entity creates
/// have the same columns, each length, precision and scale among them, so the
/// entity carries what the field type alone could not.
// [spec:pgorm:sem:codegen.entity.compact.attrs+10/test]    against a live server: the
// entity generated from a table of every kind of column type creates that table, typmod
// for typmod
// [spec:pgorm:sem:codegen.entity.types+11/test]
#[pgorm_macros::test]
async fn generated_columns_keep_their_type_modifiers() -> Result<(), Error> {
    let ctx = TestContext::new("codegen_column_types_tests_columns").await;
    let db = ctx.db.get().await?;

    db.batch_execute(SCHEMA).await?;
    let described = columns_of(&db).await?;
    db.batch_execute("DROP TABLE specimen").await?;
    db.batch_execute(
        &Schema::new()
            .create_table_from_entity(specimen::Entity)?
            .to_string(),
    )
    .await?;
    assert_eq!(columns_of(&db).await?, described);

    let typed: Vec<_> = described
        .iter()
        .map(|(name, typ, _)| (name.as_str(), typ.as_str()))
        .collect();
    assert_eq!(
        typed,
        [
            ("id", "integer"),
            ("code", "character varying(3)"),
            ("title", "character varying(255)"),
            ("note", "character varying"),
            ("initials", "character(2)"),
            ("flag", "character(1)"),
            ("body", "text"),
            ("codes", "character varying(3)[]"),
            ("marks", "character(4)[]"),
            ("notes", "character varying[]"),
            ("price", "numeric(10,2)"),
            ("whole", "numeric(6,0)"),
            ("amount", "numeric"),
            ("prices", "numeric(8,3)[]"),
            ("cost", "money"),
            ("small", "smallint"),
            ("big", "bigint"),
            ("ratio", "real"),
            ("weight", "double precision"),
            ("doc", "json"),
            ("data", "jsonb"),
            ("blob", "bytea"),
            ("ok", "boolean"),
            ("uid", "uuid"),
            ("day", "date"),
            ("clock", "time without time zone"),
            ("seen", "timestamp without time zone"),
            ("stamped", "timestamp with time zone"),
            ("wait", "interval"),
            ("months", "interval year to month"),
            ("span", "int4range"),
            ("days", "datemultirange"),
            ("level", "grade"),
            ("letters", "textrange"),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A row written through the generated entity, into the table it creates, is
/// stored as each column's type modifier makes it — a `char(n)` padded to its
/// length, a `numeric(p, s)` rounded to its scale, half away from zero — and
/// a value past a column's length or precision is refused.
// [spec:pgorm:sem:codegen.entity.compact.attrs+10/test]    against a live server: a value
// the generated entity writes is held to its column's length, precision and scale
#[pgorm_macros::test]
async fn generated_columns_hold_values_to_their_modifiers() -> Result<(), Error> {
    let ctx = TestContext::new("codegen_column_types_tests_values").await;
    let db = ctx.db.get().await?;
    entity_table(&db).await?;

    assert_eq!(Insert::one(without_cost(specimen(1))).exec(&db).await?, 1);
    type Stored = (
        String,
        String,
        String,
        Vec<String>,
        Option<Vec<String>>,
        Decimal,
        Decimal,
        Vec<Decimal>,
    );
    let stored = specimen::Entity::find_by_id(1)
        .select([
            specimen::Column::Code,
            specimen::Column::Initials,
            specimen::Column::Flag,
            specimen::Column::Codes,
            specimen::Column::Marks,
            specimen::Column::Price,
            specimen::Column::Whole,
            specimen::Column::Prices,
        ])
        .into_tuple::<Stored>()
        .one(&db)
        .await?;
    assert_eq!(
        stored,
        (
            "ABC".to_owned(),
            "A ".to_owned(),
            "Y".to_owned(),
            vec!["X".to_owned(), "YZ".to_owned()],
            Some(vec!["ab  ".to_owned()]),
            Decimal::new(101, 2),
            Decimal::new(3, 0),
            vec![Decimal::new(1001, 3)],
        )
    );

    let too_long = [
        specimen::Model {
            code: "ABCD".to_owned(),
            ..specimen(2)
        },
        specimen::Model {
            title: Some("x".repeat(256)),
            ..specimen(2)
        },
        specimen::Model {
            initials: "ABC".to_owned(),
            ..specimen(2)
        },
        specimen::Model {
            flag: "YN".to_owned(),
            ..specimen(2)
        },
        specimen::Model {
            codes: vec!["X".to_owned(), "WXYZ".to_owned()],
            ..specimen(2)
        },
        specimen::Model {
            marks: Some(vec!["abcde".to_owned()]),
            ..specimen(2)
        },
    ];
    for model in too_long {
        let error = Insert::one(without_cost(model))
            .exec(&db)
            .await
            .expect_err("a value past the column's length");
        refused_with(&error, &SqlState::STRING_DATA_RIGHT_TRUNCATION);
    }

    let too_wide = [
        specimen::Model {
            price: Decimal::new(100_000_000, 0),
            ..specimen(2)
        },
        specimen::Model {
            whole: Decimal::new(1_000_000, 0),
            ..specimen(2)
        },
        specimen::Model {
            prices: vec![Decimal::new(100_000, 0)],
            ..specimen(2)
        },
    ];
    for model in too_wide {
        let error = Insert::one(without_cost(model))
            .exec(&db)
            .await
            .expect_err("a value past the column's precision");
        refused_with(&error, &SqlState::NUMERIC_VALUE_OUT_OF_RANGE);
    }

    assert_eq!(
        specimen::Entity::find()
            .select(specimen::Column::Id)
            .into_tuple::<i32>()
            .all(&db)
            .await?,
        [1]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
