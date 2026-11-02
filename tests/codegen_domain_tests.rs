#![allow(unused_imports, dead_code)]

//! The entity pgorm-codegen generates from a table of domain columns, against
//! a live server.
//!
//! pgorm-codegen's own tests hold the writer to these files and the bridge to
//! the domain each column names. What only a server settles is that they are
//! the table's columns: the table the generated entity creates is the one the
//! schema creates, domain for domain; a value written through the entity is
//! read back as it was written, a domain's default filling a field the write
//! leaves out; and each domain's `NOT NULL` and checks refuse what they do not
//! hold, a check's refusal reaching the caller as the error PostgreSQL raised.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{Name, Range};
use pgorm::{ConnectionTrait, Schema, entity::prelude::*};
use pretty_assertions::assert_eq;
use std::ops::Bound::{Excluded, Included};
use tokio_postgres::error::SqlState;

/// The files pgorm-codegen generates from its domain schema: the ones its own
/// tests hold the writer's output to, so what is run here is what the
/// generator writes.
#[path = "../pgorm-codegen/tests/sql/domain/pgorm_active_enums.rs"]
mod pgorm_active_enums;

#[path = "../pgorm-codegen/tests/sql/domain/pgorm_range_types.rs"]
mod pgorm_range_types;

#[path = "../pgorm-codegen/tests/sql/domain/stock_line.rs"]
mod stock_line;

use pgorm_active_enums::Mood;
use pgorm_range_types::Floatrange;

const SCHEMA: &str = include_str!("../pgorm-codegen/tests/sql/domain/schema.sql");

/// The schema the file's one qualified domain lives in, which the file
/// itself does not create.
async fn schema(db: &impl ConnectionTrait) -> Result<(), Error> {
    db.batch_execute(&pgorm::pgorm_query::Schema::create(Name::runtime("billing")).to_string())
        .await?;
    db.batch_execute(SCHEMA).await
}

/// Each column of `stock_line` with its type as the server prints it and
/// whether it is `NOT NULL`.
async fn columns_of(db: &impl ConnectionTrait) -> Result<Vec<(String, String, bool)>, Error> {
    let rows = db
        .query_all(
            "SELECT attname::text, format_type(atttypid, atttypmod), attnotnull \
             FROM pg_attribute \
             WHERE attrelid = 'stock_line'::regclass AND attnum > 0 AND NOT attisdropped \
             ORDER BY attnum",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect())
}

#[track_caller]
fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

/// A check's refusal: `23514`, naming the constraint.
#[track_caller]
fn checked_by(error: &Error, constraint: &str) {
    refused_with(error, &SqlState::CHECK_VIOLATION);
    let Error::Postgres(e) = error else {
        unreachable!("refused_with matched Error::Postgres");
    };
    let db = e.as_db_error().expect("a server error");
    assert_eq!(db.constraint(), Some(constraint), "{e}");
}

fn line(id: i32) -> stock_line::Model {
    stock_line::Model {
        id,
        qty: Some(3),
        small: 40,
        code: "AB".to_owned(),
        codes: vec!["X".to_owned(), "YZ".to_owned()],
        mood: Mood::Glad,
        moods: Some(vec![Mood::Calm, Mood::Glad]),
        labels: Some(vec!["new".to_owned(), "sale".to_owned()]),
        span: Some(Floatrange(Range::new(Included(0.5), Excluded(2.5)))),
        amount: Decimal::new(1999, 2),
    }
}

/// The schema, run as written, and the table its generated entity creates
/// have the same columns, each typed with its domain, so the entity carries
/// the domain's name that its field's Rust type does not.
// [spec:pgorm:sem:codegen.entity.types+12/test]    against a live server: the entity generated
// from a table of domain columns creates that table, domain for domain
// [spec:pgorm:sem:codegen.entity.compact.attrs+12/test]
#[pgorm_macros::test]
async fn generated_domain_columns_keep_their_domains() -> Result<(), Error> {
    let ctx = TestContext::new("codegen_domain_tests_columns").await;
    let db = ctx.db.get().await?;

    schema(&db).await?;
    let described = columns_of(&db).await?;
    db.batch_execute("DROP TABLE stock_line").await?;
    db.batch_execute(
        &Schema::new()
            .create_table_from_entity(stock_line::Entity)?
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
            ("qty", "quantity"),
            ("small", "small_quantity"),
            ("code", "code"),
            ("codes", "code[]"),
            ("mood", "cheerful"),
            ("moods", "cheerful[]"),
            ("labels", "labels"),
            ("span", "span"),
            ("amount", "billing.amount"),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// Values written through the generated entity come back as written, through
/// every kind of domain the file declares; a domain's default fills the field
/// a write leaves out; and each domain refuses, by its own `NOT NULL` or
/// check, a value it does not hold.
// [spec:pgorm:sem:codegen.entity.types+12/test]    against a live server: each domain field of
// the generated entity writes and reads back, its domain's default and constraints applied
// [spec:pgorm:def:exec.decode.array+2/test]
#[pgorm_macros::test]
async fn generated_domain_fields_round_trip() -> Result<(), Error> {
    let ctx = TestContext::new("codegen_domain_tests_values").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;

    let written = line(1);
    assert_eq!(
        written.clone().into_active_model().insert(&db).await?,
        written
    );
    let empty = stock_line::Model {
        id: 2,
        moods: None,
        labels: None,
        span: None,
        codes: Vec::new(),
        ..line(2)
    };
    assert_eq!(empty.clone().into_active_model().insert(&db).await?, empty);

    let mut defaulted = line(3).into_active_model();
    defaulted.qty = NotSet;
    assert_eq!(defaulted.insert(&db).await?.qty, Some(1));

    assert_eq!(
        stock_line::Entity::find()
            .filter(stock_line::Column::Small.gt(10))
            .filter(stock_line::Column::Code.eq("AB"))
            .order_by_asc(stock_line::Column::Id)
            .all(&db)
            .await?
            .iter()
            .map(|line| line.id)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(stock_line::Entity::find_by_id(1).one(&db).await?, written);

    let null = stock_line::Model {
        qty: None,
        ..line(4)
    }
    .into_active_model()
    .insert(&db)
    .await
    .expect_err("a null quantity");
    refused_with(&null, &SqlState::NOT_NULL_VIOLATION);

    let refusals = [
        (
            stock_line::Model {
                small: 150,
                ..line(4)
            },
            "small_quantity_check",
        ),
        (
            stock_line::Model {
                qty: Some(0),
                ..line(4)
            },
            "positive",
        ),
        (
            stock_line::Model {
                code: "ab".to_owned(),
                ..line(4)
            },
            "code_check",
        ),
        (
            stock_line::Model {
                codes: vec!["X".to_owned(), "y".to_owned()],
                ..line(4)
            },
            "code_check",
        ),
        (
            stock_line::Model {
                mood: Mood::Sad,
                ..line(4)
            },
            "cheerful_check",
        ),
        (
            stock_line::Model {
                labels: Some(["a", "b", "c", "d"].map(str::to_owned).to_vec()),
                ..line(4)
            },
            "labels_check",
        ),
        (
            stock_line::Model {
                span: Some(Floatrange(Range::new(Included(1.0), Excluded(1.0)))),
                ..line(4)
            },
            "span_check",
        ),
        (
            stock_line::Model {
                amount: Decimal::new(-1, 0),
                ..line(4)
            },
            "amount_check",
        ),
    ];
    for (model, constraint) in refusals {
        let refused = model
            .into_active_model()
            .insert(&db)
            .await
            .expect_err(constraint);
        checked_by(&refused, constraint);
    }

    drop(db);
    ctx.delete().await;
    Ok(())
}
