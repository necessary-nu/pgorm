#![allow(unused_imports, dead_code)]

//! Live coverage for the scale a `numeric` keeps when it is bound as a
//! parameter rather than rendered as a literal.
//!
//! PostgreSQL carries display scale as part of a numeric's identity: `0.00`
//! and `0` are numerically equal but not the same value on the wire or in
//! output. `rust_decimal`'s `ToSql` builds its wire form through
//! `to_postgres`, which short-circuits on zero and hardcodes `scale: 0`,
//! discarding `self.scale()` — the one field a zero's encoding still has to
//! say anything with, since a zero has no digit groups at all. So `0.00`
//! reached the server as `0` while the literal path sent `0.00`. Only zero was
//! affected; every non-zero value reads its own scale on the ordinary path.
//! pgorm writes the zero header itself now, and these tests pin the two paths
//! to agree against a real server.

pub mod common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::{DecodeRaw, Schema, entity::prelude::*, set};
use pgorm_query::{Value, Values};
use pretty_assertions::assert_eq;

mod decimal_probe {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "decimal_probe")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(column_type = ColumnType::Decimal(None))]
        pub amount: Decimal,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// What `scale()` reports after the value has been to the server and back.
async fn bound_scale(db: &DatabaseConnection, value: Decimal) -> Result<u32, Error> {
    Ok(("SELECT $1::numeric", Values(vec![Value::from(value)]))
        .into_tuple::<Decimal>()
        .one(db)
        .await?
        .scale())
}

/// The reported defect: a zero lost the scale it was given, and only a zero.
// [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
#[pgorm_macros::test]
async fn a_bound_zero_keeps_its_scale() -> Result<(), Error> {
    let ctx = TestContext::new("decimal_scale_bound_zero").await;
    let db = ctx.db.get().await?;

    for scale in [0u32, 1, 2, 6, 28] {
        let value = Decimal::new(0, scale);
        assert_eq!(
            bound_scale(&db, value).await?,
            scale,
            "a zero of scale {scale}"
        );
    }

    // What PostgreSQL itself renders, which is the thing the campaign's oracle
    // compared. `0.00` had been arriving as `0`.
    for (scale, rendered) in [(0u32, "0"), (2, "0.00"), (6, "0.000000")] {
        assert_eq!(
            (
                "SELECT ($1::numeric)::text",
                Values(vec![Value::from(Decimal::new(0, scale))])
            )
                .into_tuple::<String>()
                .one(&db)
                .await?,
            rendered.to_owned(),
            "a zero of scale {scale}"
        );
    }

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The bound path and the literal path have to agree — that difference is what
/// the campaign saw. `scale` is compared explicitly because PostgreSQL's `=`
/// is numeric and would call `0` and `0.00` equal.
// [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
#[pgorm_macros::test]
async fn bound_and_literal_numerics_agree_on_scale() -> Result<(), Error> {
    let ctx = TestContext::new("decimal_scale_literal_parity").await;
    let db = ctx.db.get().await?;

    for literal in [
        "0", "0.0", "0.00", "0.000000", "1.5", "1.50", "12.3400", "0.10", "100.00", "-0.00",
        "-1.005",
    ] {
        let bound = (
            "SELECT ($1::numeric)::text",
            Values(vec![Value::from(rust_dec(literal))]),
        )
            .into_tuple::<String>()
            .one(&db)
            .await?;
        let rendered = (
            format!("SELECT (NUMERIC '{literal}')::text"),
            Values(vec![]),
        )
            .into_tuple::<String>()
            .one(&db)
            .await?;
        assert_eq!(bound, rendered, "the two paths disagree on `{literal}`");
    }

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// Non-zero values were never affected, and must stay that way.
// [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
#[pgorm_macros::test]
async fn non_zero_numerics_still_round_trip() -> Result<(), Error> {
    let ctx = TestContext::new("decimal_scale_non_zero").await;
    let db = ctx.db.get().await?;

    for literal in [
        "1.5", "1.50", "12.3400", "0.10", "100.00", "-1.005", "1", "-7",
    ] {
        let value = rust_dec(literal);
        let back = ("SELECT $1::numeric", Values(vec![Value::from(value)]))
            .into_tuple::<Decimal>()
            .one(&db)
            .await?;
        assert_eq!(back, value, "binding {literal}");
        assert_eq!(back.scale(), value.scale(), "the scale of {literal}");
    }

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// Arrays reach the same encoder element-wise through `ValueHolder`, so a
/// zero inside one keeps its scale too.
// [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
#[pgorm_macros::test]
async fn zeroes_inside_a_numeric_array_keep_their_scale() -> Result<(), Error> {
    let ctx = TestContext::new("decimal_scale_array").await;
    let db = ctx.db.get().await?;

    let values = vec![
        Decimal::new(0, 2),
        Decimal::new(0, 0),
        Decimal::new(0, 6),
        rust_dec("1.50"),
    ];

    assert_eq!(
        (
            "SELECT ($1::numeric[])::text",
            Values(vec![Value::from(values.clone())])
        )
            .into_tuple::<String>()
            .one(&db)
            .await?,
        "{0.00,0,0.000000,1.50}".to_owned()
    );

    let back = ("SELECT $1::numeric[]", Values(vec![Value::from(values)]))
        .into_tuple::<Vec<Decimal>>()
        .one(&db)
        .await?;
    assert_eq!(
        back.iter().map(Decimal::scale).collect::<Vec<_>>(),
        vec![2, 0, 6, 2]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The same value through an entity, which is how an application meets it.
// [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
#[pgorm_macros::test]
async fn an_entity_stores_zero_at_its_declared_scale() -> Result<(), Error> {
    let ctx = TestContext::new("decimal_scale_entity").await;
    let db = ctx.db.get().await?;

    let schema = Schema::new();
    create_table_without_asserts(
        &db,
        &schema.create_table_from_entity(decimal_probe::Entity)?,
    )
    .await?;

    decimal_probe::ActiveModel {
        id: set(1),
        amount: set(Decimal::new(0, 2)),
    }
    .insert(&db)
    .await?;

    let found = decimal_probe::Entity::find().one(&db).await?;
    assert_eq!(found.amount.scale(), 2);
    assert_eq!(
        ("SELECT amount::text FROM decimal_probe", Values(vec![]))
            .into_tuple::<String>()
            .one(&db)
            .await?,
        "0.00".to_owned()
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// `numeric` values no `Decimal` holds exactly, each of which rust_decimal's
/// own `FromSql` reads rounded: one with 29 fractional digits, and one whose
/// coefficient at its scale needs more than 96 bits.
const INEXACT: [&str; 2] = [
    "1.00000000000000000000000000001",
    "7922816251426433759354395033.55",
];

/// Whether `error` is `ExactDecimal`'s refusal, reached through the driver.
fn refused_as_inexact(error: &Error) -> bool {
    let text = format!("{error:?}");
    matches!(error, Error::Postgres(_))
        && (text.contains("28 fractional digits") || text.contains("96 bits"))
}

// [spec:pgorm:req:exec.decode.decimal-exact/test]    every path that decodes a
// `Decimal` refuses a `numeric` it cannot hold exactly, where rust_decimal's own
// `FromSql` would round it: a model's field, a tuple, an array, a range's bounds
#[pgorm_macros::test]
async fn an_inexact_numeric_is_refused_not_rounded() -> Result<(), Error> {
    let ctx = TestContext::new("decimal_exact_reads").await;
    let db = ctx.db.get().await?;

    let schema = Schema::new();
    create_table_without_asserts(
        &db,
        &schema.create_table_from_entity(decimal_probe::Entity)?,
    )
    .await?;

    for (id, text) in (1..).zip(INEXACT) {
        db.execute(
            "INSERT INTO decimal_probe VALUES ($1, $2::text::numeric)",
            &[&id, &text],
        )
        .await?;
        let model = decimal_probe::Entity::find_by_id(id).one(&db).await;
        assert!(
            model.as_ref().is_err_and(refused_as_inexact),
            "{text}: {model:?}"
        );
        let scalar = (
            "SELECT $1::text::numeric",
            Values(vec![Value::from(text.to_owned())]),
        )
            .into_tuple::<Decimal>()
            .one(&db)
            .await;
        assert!(
            scalar.as_ref().is_err_and(refused_as_inexact),
            "{text}: {scalar:?}"
        );
        let array = (
            "SELECT ARRAY[1.5, $1::text::numeric]",
            Values(vec![Value::from(text.to_owned())]),
        )
            .into_tuple::<Vec<Decimal>>()
            .one(&db)
            .await;
        assert!(
            array.as_ref().is_err_and(refused_as_inexact),
            "{text}: {array:?}"
        );
        let range = (
            "SELECT numrange(0, $1::text::numeric)",
            Values(vec![Value::from(text.to_owned())]),
        )
            .into_tuple::<Range<Decimal>>()
            .one(&db)
            .await;
        assert!(
            range.as_ref().is_err_and(refused_as_inexact),
            "{text}: {range:?}"
        );
    }

    // A value a `Decimal` does hold reads back exactly, scale and all.
    let held = (
        "SELECT 0.1000000000000000000000000001::numeric",
        Values(vec![]),
    )
        .into_tuple::<Decimal>()
        .one(&db)
        .await?;
    assert_eq!(held.to_string(), "0.1000000000000000000000000001");

    drop(db);
    ctx.delete().await;
    Ok(())
}
