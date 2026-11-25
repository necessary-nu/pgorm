#![allow(unused_imports, dead_code)]

//! `ALTER TYPE` on a composite's attributes against a live PostgreSQL server.
//!
//! The render tests in pgorm-query hold each change to the parser's
//! `AlterTableCmd`. What only a server settles is what the changes make — the
//! attributes the catalogue holds afterwards, in order, with their types and
//! collations — and what it refuses around them: a typed table altered
//! without `CASCADE`, a column of the type in another table, and names the
//! type has or lacks.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{ColumnType, Name, extension::Type};
use pgorm::{Connection, entity::prelude::*};
use pgorm_driver::error::SqlState;
use pretty_assertions::assert_eq;

fn n(name: &str) -> Name {
    Name::runtime(name)
}

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

/// The live attributes of the relation `relation` (a composite's row type or
/// a table), in order: name, type as `format_type` spells it, and collation
/// name where it is not the type's own.
async fn attributes(
    db: &DatabaseConnection,
    relation: &str,
) -> Result<Vec<(String, String, Option<String>)>, Error> {
    let rows = db
        .query_all(
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), \
             CASE WHEN a.attcollation <> t.typcollation THEN k.collname::text END \
             FROM pg_class c JOIN pg_attribute a ON a.attrelid = c.oid \
             JOIN pg_type t ON t.oid = a.atttypid \
             LEFT JOIN pg_collation k ON k.oid = a.attcollation \
             WHERE c.relname = $1 AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum",
            &[&relation],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect())
}

fn attribute(name: &str, ty: &str, collation: Option<&str>) -> (String, String, Option<String>) {
    (name.to_owned(), ty.to_owned(), collation.map(str::to_owned))
}

/// Changes chain in one statement and apply in order: an attribute added
/// under a collation, one dropped, one retyped, one added after it; a rename
/// is its own statement. A drop passes over a missing attribute with `IF
/// EXISTS`, where the plain drop is refused (`42703`), and an attribute added
/// twice is refused (`42701`).
// [spec:pgorm:req:sql.ddl.type-composite+3/test]    against a live server: chained changes
// and a rename make the attributes they say, and the server judges the names
#[pgorm_macros::test]
async fn attribute_changes_apply_in_order() -> Result<(), Error> {
    let ctx = TestContext::new("composite_attribute_changes").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        &Type::create(n("address"))
            .attribute(n("street"), ColumnType::Text)
            .attribute(n("no"), ColumnType::Integer)
            .to_string(),
    )
    .await?;

    let changes = Type::alter(n("address"))
        .add_attribute_collated(n("zip"), ColumnType::Text, n("C"))
        .drop_attribute(n("no"))
        .alter_attribute(n("street"), ColumnType::string(Some(200)))
        .add_attribute(n("no"), ColumnType::BigInteger)
        .drop_attribute_if_exists(n("missing"))
        .to_string();
    db.batch_execute(&changes).await?;
    db.batch_execute(
        &Type::alter(n("address"))
            .rename_attribute(n("zip"), n("postcode"))
            .to_string(),
    )
    .await?;
    assert_eq!(
        attributes(&db, "address").await?,
        [
            attribute("street", "character varying(200)", None),
            attribute("postcode", "text", Some("C")),
            attribute("no", "bigint", None),
        ],
        "{changes}"
    );

    let missing = Type::alter(n("address"))
        .drop_attribute(n("missing"))
        .to_string();
    let refused = db.batch_execute(&missing).await.expect_err(&missing);
    refused_with(&refused, &SqlState::UNDEFINED_COLUMN);
    let twice = Type::alter(n("address"))
        .add_attribute(n("street"), ColumnType::Text)
        .to_string();
    let refused = db.batch_execute(&twice).await.expect_err(&twice);
    refused_with(&refused, &SqlState::DUPLICATE_COLUMN);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A composite that is a typed table's type is altered only `CASCADE`, which
/// carries each change — an added, retyped, renamed or dropped attribute —
/// into the table's columns; without it every change is refused (`2BP01`),
/// said `RESTRICT` or not. A column of the type in another table refuses a
/// retype even with `CASCADE` (`0A000`).
// [spec:pgorm:req:sql.ddl.type-composite+3/test]    against a live server: CASCADE carries
// each change into a typed table, which refuses them otherwise
#[pgorm_macros::test]
async fn cascade_carries_changes_into_typed_tables() -> Result<(), Error> {
    let ctx = TestContext::new("composite_attribute_cascade").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        &Type::create(n("point2"))
            .attribute(n("x"), ColumnType::Integer)
            .attribute(n("y"), ColumnType::Integer)
            .to_string(),
    )
    .await?;
    db.batch_execute("CREATE TABLE located OF point2").await?;

    for refused in [
        Type::alter(n("point2"))
            .add_attribute(n("z"), ColumnType::Integer)
            .to_string(),
        Type::alter(n("point2"))
            .alter_attribute(n("x"), ColumnType::BigInteger)
            .restrict()
            .to_string(),
        Type::alter(n("point2")).drop_attribute(n("y")).to_string(),
        Type::alter(n("point2"))
            .rename_attribute(n("x"), n("across"))
            .restrict()
            .to_string(),
    ] {
        let error = db.batch_execute(&refused).await.expect_err(&refused);
        refused_with(&error, &SqlState::DEPENDENT_OBJECTS_STILL_EXIST);
    }

    db.batch_execute(
        &Type::alter(n("point2"))
            .add_attribute(n("z"), ColumnType::Integer)
            .alter_attribute(n("x"), ColumnType::BigInteger)
            .drop_attribute(n("y"))
            .cascade()
            .to_string(),
    )
    .await?;
    db.batch_execute(
        &Type::alter(n("point2"))
            .rename_attribute(n("x"), n("across"))
            .cascade()
            .to_string(),
    )
    .await?;
    let expected = [
        attribute("across", "bigint", None),
        attribute("z", "integer", None),
    ];
    assert_eq!(attributes(&db, "point2").await?, expected);
    assert_eq!(attributes(&db, "located").await?, expected);

    db.batch_execute("CREATE TABLE holder (p point2)").await?;
    let retype = Type::alter(n("point2"))
        .alter_attribute(n("z"), ColumnType::BigInteger)
        .cascade()
        .to_string();
    let refused = db.batch_execute(&retype).await.expect_err(&retype);
    refused_with(&refused, &SqlState::FEATURE_NOT_SUPPORTED);

    drop(db);
    ctx.delete().await;
    Ok(())
}
