#![allow(unused_imports, dead_code)]

//! Live coverage for the parameter wire-type check: a `Value` whose binary
//! representation is not the wire format of the type Postgres inferred for the
//! placeholder is refused client-side, before the bind message is sent.

pub mod common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::{DecodeRaw, Schema, Update, ValueHolder, entity::prelude::*, set};
use pgorm_query::{Expr, Value, Values};
use pretty_assertions::assert_eq;

mod wire_probe {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "wire_probe")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub code: i32,
        pub tally: i64,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// The message a refusal carries, so a test can tell it apart from a server
/// error that happens to have the same shape.
fn refusal(err: &Error) -> String {
    format!("{err:?}")
}

async fn seed(db: &DatabaseConnection) -> Result<(), Error> {
    let schema = Schema::new();
    create_table_without_asserts(db, &schema.create_table_from_entity(wire_probe::Entity)?).await?;

    wire_probe::ActiveModel {
        id: set(1),
        code: set(1234),
        tally: set(9),
        label: set("alpha"),
    }
    .insert(db)
    .await?;

    Ok(())
}

/// The reported defect: ASCII digits bound to an `int4` placeholder were read
/// back as the integer those four bytes spell (`"1234"` became 825373492).
// [spec:pgorm:req:exec.cursor.binding-accepts+4/test]
#[pgorm_macros::test]
async fn rejects_a_string_bound_to_an_int_placeholder() -> Result<(), Error> {
    let ctx = TestContext::new("bind_type_tests_string_as_int").await;
    let db = ctx.db.get().await?;

    let result = ("SELECT $1::int4", Values(vec!["1234".into()]))
        .into_tuple::<i32>()
        .one(&db)
        .await;

    let err = result.expect_err("a string has no int4 representation");
    assert!(
        refusal(&err).contains("cannot bind a `String` value to Postgres type `int4`"),
        "unexpected error: {err:?}"
    );

    // The control: the same query with the value the placeholder asked for.
    assert_eq!(
        ("SELECT $1::int4", Values(vec![1234_i32.into()]))
            .into_tuple::<i32>()
            .one(&db)
            .await?,
        1234
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A mismatched predicate operand must not select a row — neither the right
/// one nor, as before, a wrong one.
// [spec:pgorm:req:exec.cursor.binding-accepts+4/test]
#[pgorm_macros::test]
async fn a_mismatched_predicate_selects_nothing() -> Result<(), Error> {
    let ctx = TestContext::new("bind_type_tests_predicate").await;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    // 825373492 is what the bytes of "1234" spell as an int4; the stored code
    // is 1234. Under the old behaviour neither number was what got compared.
    let err = wire_probe::Entity::find()
        .filter(Expr::col(wire_probe::Column::Code).eq("1234"))
        .all(&db)
        .await
        .expect_err("a string operand has no int4 representation");
    assert!(
        refusal(&err).contains("cannot bind a `String` value to Postgres type `int4`"),
        "unexpected error: {err:?}"
    );

    // The row is untouched and still reachable through a well-typed predicate.
    assert_eq!(
        wire_probe::Entity::find()
            .filter(wire_probe::Column::Code.eq(1234))
            .all(&db)
            .await?
            .len(),
        1
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A mismatched write must leave the table exactly as it was.
// [spec:pgorm:req:exec.cursor.binding-accepts+4/test]
#[pgorm_macros::test]
async fn a_mismatched_write_changes_nothing() -> Result<(), Error> {
    let ctx = TestContext::new("bind_type_tests_write").await;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    let err = Update::many(wire_probe::Entity)
        .col_expr(wire_probe::Column::Code, Expr::value("9999"))
        .filter(wire_probe::Column::Id.eq(1))
        .exec(&db)
        .await
        .expect_err("a string has no int4 representation");
    assert!(
        refusal(&err).contains("cannot bind a `String` value to Postgres type `int4`"),
        "unexpected error: {err:?}"
    );

    // An INSERT bound the same way is refused before it reaches the server.
    let insert = db
        .execute(
            "INSERT INTO wire_probe (id, code, tally, label) VALUES ($1, $2, $3, $4)",
            &[
                &ValueHolder(Value::Int(Some(2))),
                &ValueHolder(Value::String(Some(Box::new("4321".to_owned())))),
                &ValueHolder(Value::BigInt(Some(1))),
                &ValueHolder(Value::String(Some(Box::new("bravo".to_owned())))),
            ],
        )
        .await
        .expect_err("a string has no int4 representation");
    assert!(
        refusal(&insert).contains("cannot bind a `String` value to Postgres type `int4`"),
        "unexpected error: {insert:?}"
    );

    assert_eq!(
        wire_probe::Entity::find().all(&db).await?,
        vec![wire_probe::Model {
            id: 1,
            code: 1234,
            tally: 9,
            label: "alpha".to_owned(),
        }]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// Every binding the check is meant to leave alone.
// [spec:pgorm:req:exec.cursor.binding-accepts+4/test]
#[pgorm_macros::test]
async fn accepts_every_supported_representation() -> Result<(), Error> {
    let ctx = TestContext::new("bind_type_tests_controls").await;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    db.batch_execute(
        "CREATE TYPE mood AS ENUM ('happy', 'sad');
         CREATE DOMAIN positive_int AS int4 CHECK (VALUE > 0);
         CREATE DOMAIN email AS text;",
    )
    .await?;

    // A string against every type whose representation is its text.
    assert_eq!(
        ("SELECT $1::text", Values(vec!["alpha".into()]))
            .into_tuple::<String>()
            .one(&db)
            .await?,
        "alpha".to_owned()
    );
    assert_eq!(
        ("SELECT ($1::mood)::text", Values(vec!["happy".into()]))
            .into_tuple::<String>()
            .one(&db)
            .await?,
        "happy".to_owned()
    );
    assert_eq!(
        (
            "SELECT ($1::email)::text",
            Values(vec!["a@b.example".into()])
        )
            .into_tuple::<String>()
            .one(&db)
            .await?,
        "a@b.example".to_owned()
    );

    // An i32 widened into an int8 placeholder, the coercion this check keeps.
    assert_eq!(
        wire_probe::Entity::find()
            .filter(wire_probe::Column::Tally.eq(9))
            .all(&db)
            .await?
            .len(),
        1
    );
    assert_eq!(
        ("SELECT $1::int8", Values(vec![9_i32.into()]))
            .into_tuple::<i64>()
            .one(&db)
            .await?,
        9
    );
    assert_eq!(
        ("SELECT $1::float8", Values(vec![9_i32.into()]))
            .into_tuple::<f64>()
            .one(&db)
            .await?,
        9.0
    );
    assert_eq!(
        ("SELECT $1::numeric", Values(vec![9_i32.into()]))
            .into_tuple::<Decimal>()
            .one(&db)
            .await?,
        rust_dec(9)
    );

    // An integer reaching a domain over int4 goes through the same coercion.
    assert_eq!(
        (
            "SELECT ($1::positive_int)::int4",
            Values(vec![7_i64.into()])
        )
            .into_tuple::<i32>()
            .one(&db)
            .await?,
        7
    );

    // The remaining payload variants against the types they belong to.
    let id = uuid::Uuid::new_v4();
    assert_eq!(
        ("SELECT $1::uuid", Values(vec![id.into()]))
            .into_tuple::<uuid::Uuid>()
            .one(&db)
            .await?,
        id
    );
    assert_eq!(
        (
            "SELECT ($1::jsonb)->>'k'",
            Values(vec![serde_json::json!({"k": "v"}).into()])
        )
            .into_tuple::<String>()
            .one(&db)
            .await?,
        "v".to_owned()
    );
    assert_eq!(
        ("SELECT $1::bytea", Values(vec![vec![1u8, 2, 3].into()]))
            .into_tuple::<Vec<u8>>()
            .one(&db)
            .await?,
        vec![1u8, 2, 3]
    );

    // Arrays are checked element-wise against the member type, so the numeric
    // coercion reaches through them too.
    assert_eq!(
        (
            "SELECT array_to_string($1::text[], ',')",
            Values(vec![vec!["a".to_owned(), "b".to_owned()].into()])
        )
            .into_tuple::<String>()
            .one(&db)
            .await?,
        "a,b".to_owned()
    );
    assert_eq!(
        (
            "SELECT array_to_string($1::int8[], ',')",
            Values(vec![vec![1_i32, 2].into()])
        )
            .into_tuple::<String>()
            .one(&db)
            .await?,
        "1,2".to_owned()
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A domain is transparent on the wire, so the decision is made against the
/// type it is built over — and a mismatch against that base is still refused.
// [spec:pgorm:req:exec.cursor.binding-accepts+4/test]
#[pgorm_macros::test]
async fn refuses_a_mismatch_through_a_domain() -> Result<(), Error> {
    let ctx = TestContext::new("bind_type_tests_domain").await;
    let db = ctx.db.get().await?;

    db.batch_execute("CREATE DOMAIN positive_int AS int4 CHECK (VALUE > 0)")
        .await?;

    let err = ("SELECT $1::positive_int", Values(vec!["7".into()]))
        .into_tuple::<i32>()
        .one(&db)
        .await
        .expect_err("a string has no int4 representation");
    assert!(
        refusal(&err).contains("cannot bind a `String` value to Postgres type `positive_int`"),
        "the refusal names the type the schema declares: {err:?}"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A `NULL` is sent as a length of -1 with no bytes, so it has no
/// representation to mismatch and binds against any inferred type.
// [spec:pgorm:req:exec.cursor.binding-accepts+4/test]
#[pgorm_macros::test]
async fn a_null_binds_against_any_type() -> Result<(), Error> {
    let ctx = TestContext::new("bind_type_tests_null").await;
    let db = ctx.db.get().await?;

    assert_eq!(
        ("SELECT $1::int4", Values(vec![Value::String(None)]))
            .into_tuple::<Option<i32>>()
            .one(&db)
            .await?,
        None
    );
    assert_eq!(
        ("SELECT $1::text", Values(vec![Value::Int(None)]))
            .into_tuple::<Option<String>>()
            .one(&db)
            .await?,
        None
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
