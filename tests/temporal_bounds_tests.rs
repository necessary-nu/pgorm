#![allow(unused_imports, dead_code)]

//! Live coverage for temporal values at the ends of their representable
//! ranges, bound as parameters rather than rendered as literals.
//!
//! The reported defect was that `9999-12-31 23:59:59.999999` could not be
//! bound at all: `postgres-types`' encoder for `jiff::civil::DateTime` derives
//! the wire payload by rounding a `Span` *relative to* a civil datetime, which
//! makes jiff project the value onto the absolute `jiff::Timestamp` timeline.
//! That timeline is narrower than the civil calendar by jiff's largest UTC
//! offset at each end, so the last day and a bit of the calendar was refused
//! with a bare `value too large to transmit` — surfacing as `error serializing
//! parameter 0` — even though PostgreSQL stores timestamps to 294276 AD and
//! the same crate's decoder reads those bytes back happily. pgorm encodes
//! `timestamp` itself now; these tests hold that against a real server.

pub mod common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::{DecodeRaw, Schema, entity::prelude::*, set};
use pgorm_query::{Value, Values};
use pretty_assertions::assert_eq;

/// The value seven defects in one campaign run shared.
fn campaign_value() -> DateTime {
    jiff::civil::date(9999, 12, 31).at(23, 59, 59, 999_999_000)
}

mod temporal_probe {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "temporal_probe")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub naive: DateTime,
        pub day: Date,
        pub clock: Time,
        pub instant: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// The defect itself, at its smallest: one parameter, one placeholder.
// [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
#[pgorm_macros::test]
async fn binds_the_maximum_precision_timestamp() -> Result<(), Error> {
    let ctx = TestContext::new("temporal_bounds_max_precision").await;
    let db = ctx.db.get().await?;

    assert_eq!(
        (
            "SELECT $1::timestamp",
            Values(vec![Value::from(campaign_value())])
        )
            .into_tuple::<DateTime>()
            .one(&db)
            .await?,
        campaign_value()
    );

    // The server agrees it is the value it was asked for, not merely some
    // value that decodes back to the same bytes.
    assert_eq!(
        (
            "SELECT ($1::timestamp)::text",
            Values(vec![Value::from(campaign_value())])
        )
            .into_tuple::<String>()
            .one(&db)
            .await?,
        "9999-12-31 23:59:59.999999".to_owned()
    );

    // And it compares equal to the literal spelling of the same instant, which
    // is the path that kept working while the bound one did not.
    assert_eq!(
        (
            "SELECT $1::timestamp = TIMESTAMP '9999-12-31 23:59:59.999999'",
            Values(vec![Value::from(campaign_value())])
        )
            .into_tuple::<bool>()
            .one(&db)
            .await?,
        true
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The whole band that shared the fault, not just its endpoint. Every one of
/// these was refused before; precision was never what distinguished them.
// [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
#[pgorm_macros::test]
async fn binds_every_naive_datetime_postgres_can_store() -> Result<(), Error> {
    let ctx = TestContext::new("temporal_bounds_naive_band").await;
    let db = ctx.db.get().await?;

    for value in [
        jiff::civil::DateTime::MAX,
        campaign_value(),
        jiff::civil::date(9999, 12, 31).at(23, 59, 59, 0),
        jiff::civil::date(9999, 12, 31).at(0, 0, 0, 0),
        // The old upper limit and the first value past it.
        jiff::civil::date(9999, 12, 30).at(22, 0, 1, 0),
        jiff::civil::date(9999, 12, 30).at(22, 0, 0, 0),
        // The epoch the wire format counts from, and either side of it.
        jiff::civil::date(2000, 1, 1).at(0, 0, 0, 0),
        jiff::civil::date(1999, 12, 31).at(23, 59, 59, 999_999_000),
        // PostgreSQL's own floor. jiff reaches further down than this, but the
        // server does not, so this is where the round trip really stops.
        jiff::civil::date(-4713, 11, 24).at(0, 0, 0, 0),
        jiff::civil::date(-4713, 11, 24).at(0, 0, 0, 1_000),
    ] {
        // `DateTime::MAX` carries a ninth fractional digit that jiff truncates
        // on write, so it lands on the microsecond below it.
        let expected = if value == jiff::civil::DateTime::MAX {
            campaign_value()
        } else {
            value
        };
        assert_eq!(
            ("SELECT $1::timestamp", Values(vec![Value::from(value)]))
                .into_tuple::<DateTime>()
                .one(&db)
                .await?,
            expected,
            "binding {value}"
        );
    }

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// Below PostgreSQL's floor the refusal has to come from the server, naming
/// the real reason, rather than from a client-side encoder that could not
/// produce the bytes. jiff's civil calendar starts at 10000 BC; PostgreSQL's
/// `timestamp` starts at 4714-11-24 BC.
// [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
#[pgorm_macros::test]
async fn reports_the_server_range_for_values_below_it() -> Result<(), Error> {
    let ctx = TestContext::new("temporal_bounds_below_server_floor").await;
    let db = ctx.db.get().await?;

    let err = (
        "SELECT $1::timestamp",
        Values(vec![Value::from(jiff::civil::DateTime::MIN)]),
    )
        .into_tuple::<DateTime>()
        .one(&db)
        .await
        .expect_err("10000 BC is below PostgreSQL's timestamp range");
    let reported = format!("{err:?}");
    assert!(
        reported.contains("timestamp out of range"),
        "expected the server's own range error, got: {reported}"
    );
    assert!(
        !reported.contains("too large to transmit"),
        "the client encoder must not be the one refusing: {reported}"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The other three temporal variants at their own extremes, so a regression in
/// any of them is visible next to the one that was fixed.
// [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
#[pgorm_macros::test]
async fn binds_the_other_temporal_variants_at_their_extremes() -> Result<(), Error> {
    let ctx = TestContext::new("temporal_bounds_other_variants").await;
    let db = ctx.db.get().await?;

    // `date` reaches 9999-12-31 at the top; the bottom is PostgreSQL's, not
    // jiff's, for the same reason as `timestamp` — 4714-11-24 BC, which is
    // -4713-11-24 in the proleptic numbering jiff uses.
    for value in [
        jiff::civil::Date::MAX,
        jiff::civil::date(2000, 1, 1),
        jiff::civil::date(-4713, 11, 24),
    ] {
        assert_eq!(
            ("SELECT $1::date", Values(vec![Value::from(value)]))
                .into_tuple::<Date>()
                .one(&db)
                .await?,
            value,
            "binding {value}"
        );
    }

    // `time` spans a whole day. `Time::MAX` has a ninth fractional digit that
    // truncates away on write.
    for (value, expected) in [
        (
            jiff::civil::Time::MAX,
            jiff::civil::time(23, 59, 59, 999_999_000),
        ),
        (jiff::civil::Time::MIN, jiff::civil::Time::MIN),
        (
            jiff::civil::time(23, 59, 59, 999_999_000),
            jiff::civil::time(23, 59, 59, 999_999_000),
        ),
    ] {
        assert_eq!(
            ("SELECT $1::time", Values(vec![Value::from(value)]))
                .into_tuple::<Time>()
                .one(&db)
                .await?,
            expected,
            "binding {value}"
        );
    }

    // `timestamptz` is `jiff::Timestamp`, whose range stops short of the civil
    // calendar at both ends — it reserves jiff's largest UTC offset so every
    // instant has a civil rendering in every zone. That is why
    // `9999-12-31 23:59:59.999999` cannot be bound as a `timestamptz` at all:
    // there is no `jiff::Timestamp` holding it, and the naive variant above is
    // the only route to that value. `Timestamp::MAX` is inside PostgreSQL's
    // range; `Timestamp::MIN` is below its floor, so the bottom of this round
    // trip is PostgreSQL's, as it is for the other two types.
    for (value, expected) in [
        (
            jiff::Timestamp::MAX,
            "9999-12-30T22:00:00.999999Z"
                .parse::<DateTimeWithTimeZone>()
                .expect("a real instant"),
        ),
        (
            "-004713-11-24T00:00:00Z"
                .parse::<DateTimeWithTimeZone>()
                .expect("a real instant"),
            "-004713-11-24T00:00:00Z"
                .parse::<DateTimeWithTimeZone>()
                .expect("a real instant"),
        ),
    ] {
        assert_eq!(
            ("SELECT $1::timestamptz", Values(vec![Value::from(value)]))
                .into_tuple::<DateTimeWithTimeZone>()
                .one(&db)
                .await?,
            expected,
            "binding {value}"
        );
    }

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The same values through an entity, which is how an application meets them:
/// written by an insert, read back by a select.
// [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
#[pgorm_macros::test]
async fn round_trips_boundary_values_through_an_entity() -> Result<(), Error> {
    let ctx = TestContext::new("temporal_bounds_entity").await;
    let db = ctx.db.get().await?;

    let schema = Schema::new();
    create_table_without_asserts(
        &db,
        &schema.create_table_from_entity(temporal_probe::Entity)?,
    )
    .await?;

    let model = temporal_probe::Model {
        id: 1,
        naive: campaign_value(),
        day: jiff::civil::Date::MAX,
        clock: jiff::civil::time(23, 59, 59, 999_999_000),
        instant: "9999-12-30T22:00:00.999999Z"
            .parse()
            .expect("inside jiff's instant range"),
    };

    let inserted = temporal_probe::ActiveModel {
        id: set(model.id),
        naive: set(model.naive),
        day: set(model.day),
        clock: set(model.clock),
        instant: set(model.instant),
    }
    .insert(&db)
    .await?;
    assert_eq!(inserted, model);

    let found = temporal_probe::Entity::find().one(&db).await?;
    assert_eq!(found, model);

    // A bound predicate on the boundary value finds it, which exercises the
    // same encoder on the read side.
    assert_eq!(
        temporal_probe::Entity::find()
            .filter(temporal_probe::Column::Naive.eq(campaign_value()))
            .all(&db)
            .await?
            .len(),
        1
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
