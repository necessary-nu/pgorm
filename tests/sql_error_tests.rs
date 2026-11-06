#![allow(unused_imports, dead_code)]

pub mod common;
pub use common::{TestContext, bakery_chain::*, setup::*};
use pgorm::error::RuntimeError;
pub use pgorm::{
    ConnectionTrait, DatabaseConnection, EntityName, entity::*, error::ConstraintViolation,
    error::Error, error::SqlError, set, tests_cfg,
};
use tokio_postgres::error::SqlState;
use uuid::Uuid;

fn sql_state(err: &Error) -> SqlState {
    match err {
        Error::Postgres(e) => e
            .code()
            .cloned()
            .unwrap_or_else(|| panic!("expected a SQLSTATE-carrying postgres error, got {e:?}")),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

#[pgorm_macros::test]
async fn main() {
    let ctx = TestContext::new("bakery_chain_sql_err_tests").await;
    create_tables(&ctx.db).await.unwrap();

    let db = ctx.db.get().await.unwrap();
    test_error(&db).await;

    drop(db);
    ctx.delete().await;
}

pub async fn test_error(db: &DatabaseConnection) {
    let mud_cake = cake::ActiveModel {
        name: set("Moldy Cake"),
        price: set(rust_dec(10.25)),
        gluten_free: set(false),
        serial: set(Uuid::new_v4()),
        bakery_id: set(None),
        ..Default::default()
    };

    let cake = mud_cake.insert(db).await.expect("could not insert cake");

    let error: Error = cake
        .into_active_model()
        .insert(db)
        .await
        .expect_err("inserting should fail due to duplicate primary key");

    assert_eq!(sql_state(&error), SqlState::UNIQUE_VIOLATION);

    let fk_cake = cake::ActiveModel {
        name: set("fk error Cake"),
        price: set(rust_dec(10.25)),
        gluten_free: set(false),
        serial: set(Uuid::new_v4()),
        bakery_id: set(Some(1000)),
        ..Default::default()
    };

    let fk_error = fk_cake
        .insert(db)
        .await
        .expect_err("create foreign key should fail with non-primary key");

    assert_eq!(sql_state(&fk_error), SqlState::FOREIGN_KEY_VIOLATION);

    let invalid_error = Error::Custom("random error".to_string());
    assert_eq!(invalid_error.sql_error(), None);
}

// [spec:pgorm:sem:error.model.sql-class+4/test]    23505 -> Unique, 23503 -> ForeignKey
#[pgorm_macros::test]
async fn sql_error_classification() {
    let ctx = TestContext::new("bakery_chain_sql_err_classification_tests").await;
    create_tables(&ctx.db).await.unwrap();

    let db = ctx.db.get().await.unwrap();

    let mud_cake = cake::ActiveModel {
        name: set("Moldy Cake"),
        price: set(rust_dec(10.25)),
        gluten_free: set(false),
        serial: set(Uuid::new_v4()),
        bakery_id: set(None),
        ..Default::default()
    };

    let cake = mud_cake.insert(&db).await.expect("could not insert cake");

    let error: Error = cake
        .into_active_model()
        .insert(&db)
        .await
        .expect_err("inserting should fail due to duplicate primary key");

    assert!(matches!(
        error.sql_error(),
        Some(SqlError::UniqueConstraintViolation(_))
    ));

    let fk_cake = cake::ActiveModel {
        name: set("fk error Cake"),
        price: set(rust_dec(10.25)),
        gluten_free: set(false),
        serial: set(Uuid::new_v4()),
        bakery_id: set(Some(1000)),
        ..Default::default()
    };

    let fk_error = fk_cake
        .insert(&db)
        .await
        .expect_err("create foreign key should fail with non-primary key");

    assert!(matches!(
        fk_error.sql_error(),
        Some(SqlError::ForeignKeyConstraintViolation(_))
    ));

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:sem:error.model.bind-refusal/test]    a value its parameter's type refuses is a
// BindRefusal naming the parameter by its placeholder and carrying pgorm's reason, its Display the
// text the bindings report, and a refusal the server makes is none
#[pgorm_macros::test]
async fn bind_refusal_names_the_parameter_and_reason() {
    use pgorm::{BindRefusal, DecodeRaw, pgorm_query::Values};

    let ctx = TestContext::new("bind_refusal").await;
    let db = ctx.db.get().await.unwrap();

    let statement = "SELECT $1::int8 AS a, $2::int4 AS n";
    let values = Values(vec![Value::from(1i64), Value::from("one")]);
    let error = (statement, values)
        .into_tuple::<(i64, i32)>()
        .one(&db)
        .await
        .expect_err("a String has no int4 to bind as");
    let refusal: BindRefusal = error.bind_refusal().expect("a bind refusal");
    assert_eq!(
        (refusal.parameter, refusal.reason.as_deref()),
        (
            2,
            Some("cannot bind a `String` value to Postgres type `int4`")
        )
    );
    assert_eq!(
        refusal.to_string(),
        "parameter $2 cannot be bound: cannot bind a `String` value to Postgres type `int4`"
    );

    let overflow = (
        "SELECT $1::int4 + 2147483647 AS n",
        Values(vec![Value::from(1i32)]),
    )
        .into_tuple::<i32>()
        .one(&db)
        .await
        .expect_err("the server refuses the overflow");
    assert_eq!(overflow.bind_refusal(), None);
    assert_eq!(Error::RecordNotFound.bind_refusal(), None);

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:sem:error.model.parameter-count/test]    a statement given another number of parameters
// than it takes is a ParameterCountRefusal holding both counts, its Display the text the bindings
// report, and no other failure is one
#[pgorm_macros::test]
async fn parameter_count_refusal_names_both_counts() {
    use pgorm::{ConnectionTrait, DecodeRaw, ParameterCountRefusal, pgorm_query::Values};

    let ctx = TestContext::new("parameter_count_refusal").await;
    let db = ctx.db.get().await.unwrap();

    let error = (
        "SELECT $1::int4 AS x, $2::int4 AS y",
        Values(vec![Value::from(1i32)]),
    )
        .into_tuple::<(i32, i32)>()
        .one(&db)
        .await
        .expect_err("one value for two placeholders");
    let refusal: ParameterCountRefusal = error.parameter_count_refusal().expect("a count refusal");
    assert_eq!((refusal.expected, refusal.given), (2, 1));
    assert_eq!(
        refusal.to_string(),
        "the statement takes 2 parameters, and 1 was given"
    );
    assert_eq!(error.bind_refusal(), None);

    let many = db
        .execute("SELECT $1::int4", &[&1i32, &2i32])
        .await
        .expect_err("two values for one placeholder");
    assert_eq!(
        many.parameter_count_refusal()
            .map(|refusal| refusal.to_string()),
        Some("the statement takes 1 parameter, and 2 were given".to_owned())
    );
    let none = db
        .execute("SELECT 1", &[&1i32])
        .await
        .expect_err("a value for no placeholder");
    assert_eq!(
        none.parameter_count_refusal()
            .map(|refusal| refusal.to_string()),
        Some("the statement takes 0 parameters, and 1 was given".to_owned())
    );
    assert_eq!(Error::RecordNotFound.parameter_count_refusal(), None);

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:sem:error.model.decode-refusal/test]    a column the driver cannot read as what was
// asked is a DecodeRefusal naming what it was reading and carrying the decoder's reason, its Display
// the text the bindings report, and a refusal the server makes or a bind refusal is none
#[pgorm_macros::test]
async fn decode_refusal_names_the_column_and_reason() {
    use pgorm::{DecodeRaw, DecodeRefusal, pgorm_query::Values};

    let ctx = TestContext::new("decode_refusal").await;
    let db = ctx.db.get().await.unwrap();

    let error = ("SELECT 1 AS one, 'x'::text AS n", Values(Vec::new()))
        .into_tuple::<(i32, i32)>()
        .one(&db)
        .await
        .expect_err("text has no i32 to read as");
    let refusal: DecodeRefusal = error.decode_refusal().expect("a decode refusal");
    let reason = "cannot convert between the Rust type `i32` and the Postgres type `text`";
    assert_eq!(
        (refusal.reading.as_str(), refusal.reason.as_deref()),
        ("error deserializing column 1", Some(reason))
    );
    assert_eq!(
        refusal.to_string(),
        format!("error deserializing column 1: {reason}")
    );

    let bind = ("SELECT $1::int4 AS n", Values(vec![Value::from("one")]))
        .into_tuple::<i32>()
        .one(&db)
        .await
        .expect_err("a String has no int4 to bind as");
    assert_eq!(bind.decode_refusal(), None);
    assert!(bind.bind_refusal().is_some());
    assert_eq!(Error::RecordNotFound.decode_refusal(), None);

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:sem:error.model.sql-class+4/test]    the payload is the server DbError's message and names
#[pgorm_macros::test]
async fn sql_error_payload_carries_server_message() {
    let ctx = TestContext::new("bakery_chain_sql_err_payload_tests").await;
    create_tables(&ctx.db).await.unwrap();

    let db = ctx.db.get().await.unwrap();

    let cake = cake::ActiveModel {
        name: set("Moldy Cake"),
        price: set(rust_dec(10.25)),
        gluten_free: set(false),
        serial: set(Uuid::new_v4()),
        bakery_id: set(None),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect("could not insert cake");

    let error = cake
        .into_active_model()
        .insert(&db)
        .await
        .expect_err("inserting should fail due to duplicate primary key");

    let Error::Postgres(driver) = &error else {
        panic!("expected Error::Postgres, got {error:?}");
    };
    let detail = driver.as_db_error().expect("a server-side error detail");
    assert_eq!(*detail.code(), SqlState::UNIQUE_VIOLATION);
    let Some(SqlError::UniqueConstraintViolation(violation)) = error.sql_error() else {
        panic!("expected a unique violation, got {:?}", error.sql_error());
    };
    assert_eq!(violation.message, detail.message());
    assert_eq!(
        names(&violation),
        [
            Some("public"),
            Some("cake"),
            None,
            None,
            detail.constraint()
        ],
        "the names are the same DbError's"
    );
    assert_eq!(
        SqlError::UniqueConstraintViolation(violation.clone()).to_string(),
        format!("Unique Constraint Violated: {}", detail.message())
    );

    drop(db);
    ctx.delete().await;
}

/// A violation's `[schema, table, column, datatype, constraint]`.
fn names(violation: &ConstraintViolation) -> [Option<&str>; 5] {
    [
        violation.schema.as_deref(),
        violation.table.as_deref(),
        violation.column.as_deref(),
        violation.datatype.as_deref(),
        violation.constraint.as_deref(),
    ]
}

/// The classification of the error `sql` fails with.
async fn refusal(db: &DatabaseConnection, sql: &str) -> SqlError {
    let error = db
        .execute(sql, &[])
        .await
        .expect_err("the server refuses the statement");
    error
        .sql_error()
        .unwrap_or_else(|| panic!("a constraint violation, got {error:?}"))
}

// [spec:pgorm:sem:error.model.sql-class+4/test]    23514 and 23502, from a table and from a domain
#[pgorm_macros::test]
async fn check_and_not_null_violations_carry_names() {
    let ctx = TestContext::new("sql_err_check_not_null_tests").await;
    let db = ctx.db.get().await.unwrap();
    db.batch_execute(
        "CREATE DOMAIN positive AS int4 NOT NULL CONSTRAINT positive_check CHECK (VALUE > 0);
         CREATE TABLE gauge (
             id int4 PRIMARY KEY,
             reading int4 NOT NULL CONSTRAINT reading_bounded CHECK (reading < 100),
             level positive
         );",
    )
    .await
    .unwrap();

    let SqlError::NotNullConstraintViolation(column) = refusal(
        &db,
        "INSERT INTO gauge (id, reading, level) VALUES (1, NULL, 1)",
    )
    .await
    else {
        panic!("a NOT NULL column refuses a null as a not-null violation");
    };
    assert_eq!(
        names(&column),
        [Some("public"), Some("gauge"), Some("reading"), None, None]
    );
    assert!(column.message.contains("violates not-null constraint"));

    let SqlError::CheckConstraintViolation(check) = refusal(
        &db,
        "INSERT INTO gauge (id, reading, level) VALUES (1, 100, 1)",
    )
    .await
    else {
        panic!("a table's CHECK refuses a row as a check violation");
    };
    assert_eq!(
        names(&check),
        [
            Some("public"),
            Some("gauge"),
            None,
            None,
            Some("reading_bounded")
        ]
    );

    let SqlError::CheckConstraintViolation(domain_check) = refusal(
        &db,
        "INSERT INTO gauge (id, reading, level) VALUES (1, 1, 0)",
    )
    .await
    else {
        panic!("a domain's CHECK refuses a value as a check violation");
    };
    assert_eq!(
        names(&domain_check),
        [
            Some("public"),
            None,
            None,
            Some("positive"),
            Some("positive_check")
        ]
    );

    let SqlError::NotNullConstraintViolation(domain_null) =
        refusal(&db, "INSERT INTO gauge (id, reading) VALUES (1, 1)").await
    else {
        panic!("a NOT NULL domain refuses the null its column defaults to");
    };
    assert_eq!(
        names(&domain_null),
        [Some("public"), None, None, Some("positive"), None]
    );
    assert_eq!(
        SqlError::NotNullConstraintViolation(domain_null).to_string(),
        "Not-Null Constraint Violated: domain positive does not allow null values"
    );

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:sem:error.model.sql-class+4/test]    23P01, from an EXCLUDE and a WITHOUT OVERLAPS key
#[pgorm_macros::test]
async fn exclusion_violations_carry_their_constraint() {
    let ctx = TestContext::new("sql_err_exclusion_tests").await;
    let db = ctx.db.get().await.unwrap();
    db.batch_execute(
        "CREATE EXTENSION IF NOT EXISTS btree_gist;
         CREATE TABLE slot (
             span int4range,
             CONSTRAINT slot_apart EXCLUDE USING gist (span WITH &&)
         );
         CREATE TABLE stay (
             room int4,
             during daterange,
             CONSTRAINT stay_pkey PRIMARY KEY (room, during WITHOUT OVERLAPS)
         );
         INSERT INTO slot VALUES ('[1,5)');
         INSERT INTO stay VALUES (1, '[2026-01-01,2026-02-01)');",
    )
    .await
    .unwrap();

    let SqlError::ExclusionConstraintViolation(exclude) =
        refusal(&db, "INSERT INTO slot VALUES ('[4,8)')").await
    else {
        panic!("an overlapping span is an exclusion violation");
    };
    assert_eq!(
        names(&exclude),
        [Some("public"), Some("slot"), None, None, Some("slot_apart")]
    );

    let SqlError::ExclusionConstraintViolation(temporal) = refusal(
        &db,
        "INSERT INTO stay VALUES (1, '[2026-01-15,2026-03-01)')",
    )
    .await
    else {
        panic!("an overlapping period under a WITHOUT OVERLAPS key is an exclusion violation");
    };
    assert_eq!(
        names(&temporal),
        [Some("public"), Some("stay"), None, None, Some("stay_pkey")]
    );
    assert!(
        SqlError::ExclusionConstraintViolation(temporal)
            .to_string()
            .starts_with("Exclusion Constraint Violated: conflicting key value")
    );

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:sem:error.model.sql-class+4/test]    every other error classifies as None
#[pgorm_macros::test]
async fn sql_error_none_for_unclassified_errors() {
    let ctx = TestContext::new("bakery_chain_sql_err_none_tests").await;
    let db = ctx.db.get().await.unwrap();

    // A different SQLSTATE: still `Error::Postgres`, still not a constraint violation.
    let undefined_table = db
        .query_one("SELECT id FROM absent_table", &[])
        .await
        .expect_err("there is no such table");
    assert_eq!(sql_state(&undefined_table), SqlState::UNDEFINED_TABLE);
    assert_eq!(undefined_table.sql_error(), None);

    // An `Error::Postgres` with no server-side `DbError`: the parameter cannot be
    // encoded, so the driver fails before the statement reaches the server.
    let client_side = db
        .query_one("SELECT $1::int4", &[&"not an int"])
        .await
        .expect_err("a string cannot be bound to an int4 parameter");
    let Error::Postgres(driver) = &client_side else {
        panic!("expected Error::Postgres, got {client_side:?}");
    };
    assert!(
        driver.as_db_error().is_none(),
        "this failure carries no server detail: {driver:?}"
    );
    assert_eq!(client_side.sql_error(), None);

    // Every non-`Postgres` variant is unclassifiable.
    for err in [
        Error::Custom("random error".to_owned()),
        Error::RecordNotFound,
        Error::RecordNotInserted,
        Error::Query(RuntimeError::Internal("no socket".to_owned())),
        Error::Type("not a date".to_owned()),
    ] {
        assert_eq!(err.sql_error(), None, "{err:?}");
    }

    drop(db);
    ctx.delete().await;
}
