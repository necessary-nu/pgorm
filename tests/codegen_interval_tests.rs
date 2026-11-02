#![allow(unused_imports, dead_code)]

//! The entity pgorm-codegen generates from a table of interval columns, one
//! for each kind of qualifier, against a live server.
//!
//! pgorm-codegen's own tests hold the writer to this file and the bridge to
//! the qualifier each column spells. What only a server settles is that they
//! are the table's columns: the table the generated entity creates is the one
//! the schema creates, typmod for typmod, and a value written through the
//! entity is stored as the column's qualifier makes it — the fields a
//! qualifier leaves out dropped, the precision it names rounded to — and read
//! back as the server stored it.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::{ConnectionTrait, Schema, entity::prelude::*};
use pretty_assertions::assert_eq;

/// The entity pgorm-codegen generates from its interval schema: the file its
/// own tests hold the writer's output to, so what is run here is what the
/// generator writes.
#[path = "../pgorm-codegen/tests/sql/interval/interval_span.rs"]
mod interval_span;

const SCHEMA: &str = include_str!("../pgorm-codegen/tests/sql/interval/schema.sql");

/// Each column of `interval_span` with its type as the server prints it,
/// typmod included, and whether it is `NOT NULL`.
async fn columns_of(db: &DatabaseConnection) -> Result<Vec<(String, String, bool)>, Error> {
    let rows = db
        .query_all(
            "SELECT attname::text, format_type(atttypid, atttypmod), attnotnull \
             FROM pg_attribute \
             WHERE attrelid = 'interval_span'::regclass AND attnum > 0 AND NOT attisdropped \
             ORDER BY attnum",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect())
}

const HOUR: i64 = 3_600_000_000;
const MINUTE: i64 = 60_000_000;

/// One year two months, minus three days, minus four hours and 1.234567
/// seconds: a value every qualifier changes, the time below zero so the
/// truncations and roundings are seen toward zero and away from it.
fn behind() -> Interval {
    Interval::new(14, -3, -(4 * HOUR + 1_234_567))
}

/// Minus one month, plus two days, one hour one minute and 3.000004 seconds.
fn ahead() -> Interval {
    Interval::new(-1, 2, HOUR + MINUTE + 3_000_004)
}

/// The row written through the entity for `value`.
fn written(id: i32, value: Interval) -> interval_span::Model {
    interval_span::Model {
        id,
        plain: value,
        maybe: Some(value),
        precise: value,
        months: value,
        days: value,
        clock: Some(value),
        minutes: value,
        seconds: value,
        history: vec![behind(), ahead()],
        budgets: Some(vec![behind(), ahead()]),
    }
}

/// The schema, run as written, and the table its generated entity creates
/// have the same columns, each qualifier and precision among them, so the
/// entity carries what the field type alone could not.
// [spec:pgorm:sem:codegen.entity.types+11/test]    against a live server: the
// entity generated from a table of interval columns creates that table,
// qualifier for qualifier
// [spec:pgorm:sem:codegen.entity.compact.attrs+10/test]
#[pgorm_macros::test]
async fn generated_interval_columns_keep_their_qualifiers() -> Result<(), Error> {
    let ctx = TestContext::new("codegen_interval_tests_qualifiers").await;
    let db = ctx.db.get().await?;

    db.batch_execute(SCHEMA).await?;
    let described = columns_of(&db).await?;
    db.batch_execute("DROP TABLE interval_span").await?;
    db.batch_execute(
        &Schema::new()
            .create_table_from_entity(interval_span::Entity)?
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
            ("plain", "interval"),
            ("maybe", "interval"),
            ("precise", "interval(3)"),
            ("months", "interval year to month"),
            ("days", "interval day"),
            ("clock", "interval day to second(3)"),
            ("minutes", "interval hour to minute"),
            ("seconds", "interval second(0)"),
            ("history", "interval[]"),
            ("budgets", "interval year to month[]"),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// Mixed-sign values written through the generated entity, into the table it
/// creates, come back as each column stores them: the unqualified columns
/// exactly, the qualified ones with the fields their qualifier leaves out
/// dropped and the seconds rounded to their precision, half away from zero.
// [spec:pgorm:sem:codegen.entity.types+11/test]    against a live server: an
// interval field of the generated entity writes and reads back
#[pgorm_macros::test]
async fn generated_interval_fields_round_trip() -> Result<(), Error> {
    let ctx = TestContext::new("codegen_interval_tests_values").await;
    let db = ctx.db.get().await?;

    db.batch_execute(
        &Schema::new()
            .create_table_from_entity(interval_span::Entity)?
            .to_string(),
    )
    .await?;

    let stored = |id: i32, value: Interval| {
        let Interval {
            months,
            days,
            microseconds,
        } = value;
        let to_millis = |micros: i64| (micros + micros.signum() * 500) / 1_000 * 1_000;
        let to_seconds = |micros: i64| (micros + micros.signum() * 500_000) / 1_000_000 * 1_000_000;
        let year_to_month = |value: Interval| Interval::new(value.months, 0, 0);
        interval_span::Model {
            id,
            plain: value,
            maybe: Some(value),
            precise: Interval::new(months, days, to_millis(microseconds)),
            months: Interval::new(months, 0, 0),
            days: Interval::new(months, days, 0),
            clock: Some(Interval::new(months, days, to_millis(microseconds))),
            minutes: Interval::new(months, days, microseconds / MINUTE * MINUTE),
            seconds: Interval::new(months, days, to_seconds(microseconds)),
            history: vec![behind(), ahead()],
            budgets: Some(vec![year_to_month(behind()), year_to_month(ahead())]),
        }
    };
    assert_eq!(
        stored(1, behind()).precise,
        Interval::new(14, -3, -(4 * HOUR + 1_235_000))
    );
    assert_eq!(
        stored(1, behind()).minutes,
        Interval::new(14, -3, -4 * HOUR)
    );
    assert_eq!(
        stored(2, ahead()).seconds,
        Interval::new(-1, 2, HOUR + MINUTE + 3_000_000)
    );

    for (id, value) in [(1, behind()), (2, ahead())] {
        let inserted = written(id, value).into_active_model().insert(&db).await?;
        assert_eq!(inserted, stored(id, value), "returned for {value}");
    }
    let empty = interval_span::Model {
        id: 3,
        maybe: None,
        clock: None,
        history: Vec::new(),
        budgets: None,
        ..written(3, Interval::default())
    };
    assert_eq!(empty.clone().into_active_model().insert(&db).await?, empty);

    assert_eq!(
        interval_span::Entity::find()
            .order_by_asc(interval_span::Column::Id)
            .all(&db)
            .await?,
        [stored(1, behind()), stored(2, ahead()), empty]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
