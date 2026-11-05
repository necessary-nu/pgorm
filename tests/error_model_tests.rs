#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, bakery_chain::*, setup::*};

use pgorm::{ColumnFromStrError, Error, RuntimeError, entity::prelude::*};
use pgorm::{PoolError, Runtime};
use pretty_assertions::assert_eq;
use std::{error::Error as StdError, fmt, str::FromStr, time::Duration};
use tokio_postgres::error::SqlState;

#[derive(Debug)]
struct Boom(&'static str);

impl fmt::Display for Boom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl StdError for Boom {}

// [spec:pgorm:def:error.model+12/test]    the variants pgorm constructs itself, and how each renders
#[test]
fn error_variants_render_expected_messages() {
    let cases: Vec<(Error, &str)> = vec![
        (
            Error::Conversion {
                from: "i64",
                into: "u8",
                source: Box::new(Boom("out of range")),
            },
            "Error converting `i64` into `u8`: out of range",
        ),
        (
            Error::Query(RuntimeError::Internal("bad filter".to_owned())),
            "Query Error: bad filter",
        ),
        (
            Error::ConvertFromU64("Uuid"),
            "Type 'Uuid' cannot be converted from u64",
        ),
        (
            Error::UnpackInsertId,
            "Failed to unpack the inserted primary key",
        ),
        (Error::PrimaryKeyNotSet, "A primary key value is not set"),
        (
            Error::AttrNotSet("name".to_owned()),
            "Attribute name is NotSet",
        ),
        (
            Error::Type("not a date".to_owned()),
            "Type Error: not a date",
        ),
        (
            Error::Json("not an object".to_owned()),
            "Json Error: not an object",
        ),
        (
            Error::RecordNotFound,
            "No records were returned for the given query",
        ),
        (Error::RecordNotInserted, "None of the records are inserted"),
        (Error::NothingToSet, "The update has no columns to set"),
        (Error::Custom("boom".to_owned()), "Custom Error: boom"),
    ];

    for (err, rendered) in cases {
        assert_eq!(err.to_string(), rendered);
    }
}

/// A closure error of the caller's own that converts into pgorm's.
#[derive(Debug)]
struct Refusal;

impl From<Refusal> for Error {
    fn from(_: Refusal) -> Self {
        Error::Custom("refused".to_owned())
    }
}

// [spec:pgorm:sem:conn.tx.compose/test]    Connection and Transaction flatten; RollbackFailed keeps both errors
// [spec:pgorm:def:error.model+12/test]    RollbackFailed renders both errors and sources the cause
#[test]
fn transaction_error_converts_into_error() {
    use pgorm::TransactionError;

    let connection = Error::from(TransactionError::<Error>::Connection(Error::NothingToSet));
    assert!(matches!(connection, Error::NothingToSet), "{connection:?}");
    let closure = Error::from(TransactionError::Transaction(Error::RecordNotFound));
    assert!(matches!(closure, Error::RecordNotFound), "{closure:?}");
    let converted = Error::from(TransactionError::Transaction(Refusal));
    assert!(matches!(converted, Error::Custom(ref message) if message == "refused"));

    let both = Error::from(TransactionError::RollbackFailed {
        cause: Refusal,
        rollback: Error::PrimaryKeyNotSet,
    });
    assert!(
        matches!(
            &both,
            Error::RollbackFailed { cause, rollback }
                if matches!(**cause, Error::Custom(_)) && matches!(**rollback, Error::PrimaryKeyNotSet)
        ),
        "{both:?}"
    );
    assert_eq!(
        both.to_string(),
        "Custom Error: refused; its ROLLBACK failed: A primary key value is not set"
    );
    assert_eq!(
        StdError::source(&both).map(ToString::to_string),
        Some("Custom Error: refused".to_owned())
    );
    assert!(!both.is_retryable());
}

// [spec:pgorm:def:error.model+12/test]    PartialEq/Eq compare rendered messages, not payloads
#[test]
fn error_eq_compares_rendered_messages() {
    fn assert_is_eq<T: Eq>() {}
    assert_is_eq::<Error>();

    let from_io = Error::Conversion {
        from: "i64",
        into: "u8",
        source: Box::new(std::io::Error::other("out of range")),
    };
    let from_boom = Error::Conversion {
        from: "i64",
        into: "u8",
        source: Box::new(Boom("out of range")),
    };
    assert_eq!(
        from_io, from_boom,
        "distinct payloads with identical rendered messages compare equal"
    );

    assert_ne!(Error::Custom("a".to_owned()), Error::Custom("b".to_owned()));
    assert_ne!(Error::RecordNotFound, Error::RecordNotInserted);
    assert_eq!(Error::RecordNotFound, Error::RecordNotFound);
}

// [spec:pgorm:def:error.model+12/test]    ColumnFromStrError covers FromStr failures on entity columns
#[test]
fn column_from_str_error_reports_bad_input() {
    assert!(matches!(
        cake::Column::from_str("gluten_free"),
        Ok(cake::Column::GlutenFree)
    ));

    let err: ColumnFromStrError =
        cake::Column::from_str("not_a_column").expect_err("there is no such column");
    assert_eq!(err.0, "not_a_column");
    assert_eq!(
        err.to_string(),
        r#"Failed to match "not_a_column" as Column"#
    );
}

// [spec:pgorm:def:error.model+12/test]    Result defaults to Error but still takes a foreign one
#[test]
fn result_alias_defaults_to_error() {
    fn defaulted() -> pgorm::Result<u8> {
        Err(Error::RecordNotFound)
    }

    fn foreign() -> pgorm::Result<u8, Boom> {
        Err(Boom("not ours"))
    }

    let widened: std::result::Result<u8, Error> = defaulted();
    assert_eq!(
        widened.expect_err("the alias resolves to Error"),
        Error::RecordNotFound
    );
    assert_eq!(foreign().expect_err("the alias carries Boom").0, "not ours");
}

// [spec:pgorm:def:error.model.runtime+3/test]    Internal is the only variant, and the payload of Query
#[test]
fn runtime_error_wraps_internal_message() {
    let runtime = RuntimeError::Internal("boom".to_owned());
    // Irrefutable: this only compiles while `Internal` is the sole variant.
    let RuntimeError::Internal(message) = &runtime;
    assert_eq!(message, "boom");
    assert_eq!(runtime.to_string(), "boom");

    let err = Error::Query(RuntimeError::Internal("boom".to_owned()));
    let Error::Query(inner) = &err else {
        panic!("expected a RuntimeError-carrying variant, got {err:?}");
    };
    let RuntimeError::Internal(message) = inner;
    assert_eq!(message, "boom");
    assert!(
        StdError::source(&err).is_some(),
        "the RuntimeError is the error source: {err:?}"
    );
}

// [spec:pgorm:def:error.model.runtime+3/test]    a real internal failure arrives as Query(Internal(..))
#[pgorm_macros::test]
async fn query_err_surfaces_through_loader_misuse() -> Result<(), Error> {
    let ctx = TestContext::new("error_model_runtime_errmodel").await;
    let db = ctx.db.get().await?;

    // `cake -> lineitem` is HasMany, so asking the loader for one is a pgorm-internal
    // misuse, reported through the crate-private `query_err` helper.
    let err = Vec::<cake::Model>::new()
        .load_one(Lineitem, &db)
        .await
        .expect_err("cake has many lineitems");

    let Error::Query(RuntimeError::Internal(message)) = &err else {
        panic!("expected Error::Query(RuntimeError::Internal(..)), got {err:?}");
    };
    assert_eq!(message, "Relation is HasMany instead of HasOne");
    assert_eq!(
        err.to_string(),
        "Query Error: Relation is HasMany instead of HasOne"
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:error.model+12/test]    ConnectionTrait failures arrive as Postgres, rendering the server detail
#[pgorm_macros::test]
async fn error_postgres_carries_server_detail() -> Result<(), Error> {
    let ctx = TestContext::new("error_model_postgres_errmodel").await;
    let db = ctx.db.get().await?;

    let err = db
        .query_one("SELECT id FROM absent_table", &[])
        .await
        .expect_err("there is no such table");

    let Error::Postgres(driver) = &err else {
        panic!("expected Error::Postgres, got {err:?}");
    };
    let detail = driver
        .as_db_error()
        .expect("the server reported an error detail");
    assert_eq!(*detail.code(), SqlState::UNDEFINED_TABLE);

    let rendered = err.to_string();
    assert!(rendered.starts_with("Postgres Error:"), "{rendered}");
    // The server-side `DbError` is rendered (debug-formatted, hence the escaped quotes).
    assert!(
        rendered.contains(&format!("message: {:?}", detail.message())),
        "the Display carries the server detail: {rendered}"
    );
    assert!(rendered.contains("SqlState(E42P01)"), "{rendered}");

    // The other ConnectionTrait entry points converge on the same variant.
    for other in [
        db.execute("INSERT INTO absent_table (id) VALUES (1)", &[])
            .await
            .map(|_| ())
            .expect_err("execute fails the same way"),
        db.batch_execute("SELECT id FROM absent_table")
            .await
            .expect_err("batch_execute fails the same way"),
        db.query_all("SELECT id FROM absent_table", &[])
            .await
            .map(|_| ())
            .expect_err("query_all fails the same way"),
    ] {
        assert!(matches!(other, Error::Postgres(_)), "{other:?}");
    }

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:error.model+12/test]    DatabasePool::get surfaces pool exhaustion as Error::Pool
#[pgorm_macros::test]
async fn error_pool_from_acquisition_timeout() -> Result<(), Error> {
    let ctx = TestContext::new("error_model_pool_errmodel").await;
    let base_url = std::env::var("DATABASE_URL").expect("DATABASE_URL is set by TestContext");

    let pool = pgorm::connect_with_builder(config(&base_url, ctx.db_name()), |builder| {
        builder
            .max_size(1)
            .wait_timeout(Some(Duration::from_millis(100)))
            .runtime(Runtime::Tokio1)
    })?;

    let held = pool.get().await?;
    let err = pool
        .get()
        .await
        .expect_err("the pool's only connection is checked out");

    assert!(
        matches!(err, Error::Pool(PoolError::Timeout(_))),
        "expected a wait timeout, got {err:?}"
    );
    assert!(err.to_string().starts_with("Pool Error:"), "{err}");

    drop(held);
    let recovered = pool.get().await?;
    drop(recovered);

    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:error.model+12/test]    the builder layer's refusals convert
// in via `From`, so `?` crosses the crate boundary without a `map_err`
#[test]
fn query_builder_errors_convert_via_from() {
    fn build() -> Result<pgorm::pgorm_query::SqlTemplate, Error> {
        Ok(pgorm::pgorm_query::SqlTemplate::new("$1 <-> $3", [])?)
    }

    let err = build().expect_err("a placeholder past the supplied values");
    assert!(matches!(err, Error::QueryBuilder(_)), "got {err:?}");
    assert!(
        err.to_string().starts_with("Query Builder Error: "),
        "got: {err}"
    );
}
