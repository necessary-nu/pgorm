#![allow(unused_imports, dead_code)]

//! Live coverage for `interval` values: bound in binary through `ValueHolder`,
//! inlined as literals, decoded through `TryGetable`, and written through an
//! entity's `ActiveValue` — each holding months, days and microseconds apart.

pub mod common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::{DecodeRaw, Schema, entity::prelude::*, set};
use pgorm_query::{Query, Value, Values};
use pretty_assertions::assert_eq;

mod interval_probe {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "interval_probe")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub span: Interval,
        pub maybe: Option<Interval>,
        pub spans: Vec<Interval>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// Mixed signs, zero, each field at both of its limits, the finite triples
/// beside the infinities, and the two infinities themselves.
fn cases() -> Vec<Interval> {
    vec![
        Interval::new(1, -2, 3_000_004),
        Interval::new(-1, 2, -3_000_004),
        Interval::new(14, -3, -(4 * 3_600_000_000 + 1)),
        Interval::default(),
        Interval::new(0, 0, 1),
        Interval::new(0, 0, -1),
        Interval::new(i32::MAX, 0, 0),
        Interval::new(i32::MIN, 0, 0),
        Interval::new(0, i32::MAX, 0),
        Interval::new(0, i32::MIN, 0),
        Interval::new(0, 0, i64::MAX),
        Interval::new(0, 0, i64::MIN),
        Interval::new(i32::MIN, i32::MAX, i64::MIN),
        Interval::new(i32::MAX, i32::MAX, i64::MAX - 1),
        Interval::new(i32::MIN, i32::MIN, i64::MIN + 1),
        Interval::new(i32::MAX, i32::MAX, i64::MAX),
        Interval::new(i32::MIN, i32::MIN, i64::MIN),
    ]
}

async fn bound(db: &DatabaseConnection, interval: Interval) -> Result<Interval, Error> {
    ("SELECT $1::interval", Values(vec![interval.into()]))
        .into_tuple::<Interval>()
        .one(db)
        .await
}

async fn inlined(db: &DatabaseConnection, interval: Interval) -> Result<Interval, Error> {
    let sql = format!("SELECT {}", Value::from(interval));
    (sql.as_str(), Values(Vec::new()))
        .into_tuple::<Interval>()
        .one(db)
        .await
}

// [spec:pgorm:req:exec.cursor.binding-interval/test]
// [spec:pgorm:req:exec.decode.interval+1/test]
#[pgorm_macros::test]
async fn bound_intervals_round_trip_exactly() -> Result<(), Error> {
    let ctx = TestContext::new("interval_tests_bound").await;
    let db = ctx.db.get().await?;

    for interval in cases() {
        assert_eq!(bound(&db, interval).await?, interval, "binding {interval}");
    }

    // The server reads the three fields it was sent, not a normalised value:
    // a month stays a month beside negative days, and the microseconds keep
    // their last digit.
    assert_eq!(
        (
            "SELECT ($1::interval)::text",
            Values(vec![Interval::new(1, -2, 3_000_004).into()])
        )
            .into_tuple::<String>()
            .one(&db)
            .await?,
        "1 mon -2 days +00:00:03.000004".to_owned()
    );
    assert_eq!(
        (
            "SELECT ($1::interval)::text, ($2::interval)::text",
            Values(vec![
                Interval::new(i32::MAX, i32::MAX, i64::MAX).into(),
                Interval::new(i32::MIN, i32::MIN, i64::MIN).into(),
            ])
        )
            .into_tuple::<(String, String)>()
            .one(&db)
            .await?,
        ("infinity".to_owned(), "-infinity".to_owned())
    );

    // A NULL binds against `interval` and decodes as no interval.
    assert_eq!(
        ("SELECT $1::interval", Values(vec![Value::Interval(None)]))
            .into_tuple::<Option<Interval>>()
            .one(&db)
            .await?,
        None
    );

    // An array keeps each element's fields, in order.
    assert_eq!(
        ("SELECT $1::interval[]", Values(vec![cases().into()]))
            .into_tuple::<Vec<Interval>>()
            .one(&db)
            .await?,
        cases()
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:sql.value.interval-text+1/test]
#[pgorm_macros::test]
async fn inlined_intervals_read_back_as_bound_ones() -> Result<(), Error> {
    let ctx = TestContext::new("interval_tests_inlined").await;
    let db = ctx.db.get().await?;

    for interval in cases() {
        assert_eq!(
            inlined(&db, interval).await?,
            interval,
            "inlining {interval}"
        );
    }

    // Under `sql_standard`, a leading `-` applies to every field after it
    // unless another field carries a sign of its own, which every field the
    // literal writes does.
    db.batch_execute("SET intervalstyle = sql_standard").await?;
    for interval in cases() {
        assert_eq!(
            inlined(&db, interval).await?,
            interval,
            "inlining {interval} under sql_standard"
        );
    }

    // Each element of an inlined array is cast, so the array is an
    // `interval[]` a column takes as it is, where `text[]` would be refused.
    db.batch_execute("RESET intervalstyle; CREATE TABLE spans (span interval[])")
        .await?;
    let insert = Query::insert()
        .into_table(pgorm_query::Name::runtime("spans"))
        .columns([pgorm_query::Name::runtime("span")])
        .values_panic([cases().into()])
        .to_string();
    db.batch_execute(&insert).await?;
    assert_eq!(
        ("SELECT span FROM spans", Values(Vec::new()))
            .into_tuple::<Vec<Interval>>()
            .one(&db)
            .await?,
        cases()
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:exec.cursor.binding-interval/test]
#[pgorm_macros::test]
async fn interval_bound_to_another_type_is_refused() -> Result<(), Error> {
    let ctx = TestContext::new("interval_tests_refused").await;
    let db = ctx.db.get().await?;

    for target in ["int8", "text", "time"] {
        let sql = format!("SELECT $1::{target} IS NULL");
        let err = (sql.as_str(), Values(vec![Interval::new(0, 1, 0).into()]))
            .into_tuple::<bool>()
            .one(&db)
            .await
            .expect_err("an interval has no representation in another type");
        assert!(
            format!("{err:?}").contains(&format!(
                "cannot bind a `Interval` value to Postgres type `{target}`"
            )),
            "{err:?}"
        );
    }

    // A domain is sent as the type it is built over.
    db.batch_execute("CREATE DOMAIN span AS interval").await?;
    assert_eq!(
        (
            "SELECT $1::span::interval",
            Values(vec![Interval::new(1, -2, 3).into()])
        )
            .into_tuple::<Interval>()
            .one(&db)
            .await?,
        Interval::new(1, -2, 3)
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:exec.decode.interval+1/test]
#[pgorm_macros::test]
async fn entity_interval_fields_write_and_read() -> Result<(), Error> {
    let ctx = TestContext::new("interval_tests_entity").await;
    let db = ctx.db.get().await?;

    let schema = Schema::new();
    create_table_without_asserts(
        &db,
        &schema.create_table_from_entity(interval_probe::Entity)?,
    )
    .await?;

    let models: Vec<interval_probe::Model> = cases()
        .into_iter()
        .enumerate()
        .map(|(id, span)| interval_probe::Model {
            id: id as i32,
            span,
            maybe: (id % 2 == 0).then_some(Interval::new(-(id as i32), id as i32, -1)),
            spans: vec![span, Interval::new(1, -2, 3)],
        })
        .collect();
    for model in &models {
        assert_eq!(model.clone().into_active_model().insert(&db).await?, *model);
    }
    assert_eq!(
        interval_probe::Entity::find()
            .order_by_asc(interval_probe::Column::Id)
            .all(&db)
            .await?,
        models
    );

    // A predicate binds the interval it compares with. PostgreSQL's `=` on
    // `interval` holds 1 mon equal to 30 days, so the probe is a value with
    // no other spelling among the rows.
    let found = interval_probe::Entity::find()
        .filter(interval_probe::Column::Span.eq(Interval::new(14, -3, -(4 * 3_600_000_000 + 1))))
        .one(&db)
        .await?;
    assert_eq!(found.id, 2);

    let updated = interval_probe::ActiveModel {
        id: set(0),
        span: set(Interval::new(-7, 8, -9)),
        maybe: set(None),
        ..Default::default()
    }
    .update(&db)
    .await?;
    assert_eq!(
        (updated.span, updated.maybe),
        (Interval::new(-7, 8, -9), None)
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
