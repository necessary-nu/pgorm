#![allow(unused_imports, dead_code)]

//! `CREATE TYPE ... AS (composite)` against a live PostgreSQL server.
//!
//! The render tests in pgorm-query hold the statement to libpg_query's
//! `CompositeTypeStmt`. What only a server settles is what it made: a row
//! type whose attributes are the ones written, in order, each with the type
//! and collation it was given; a type a table column can then be declared
//! with and a row written into; that the existing `DROP TYPE` and `RENAME TO`
//! reach a composite as they reach an enumeration while the label alterations
//! do not; and what still does not read a composite value back.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{ColumnDef, ColumnType, Name, Table, Values, extension::Type};
use pgorm::{ConnectionTrait, SelectGetableTuple, SelectorRaw, entity::prelude::*};
use std::error::Error as _;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::WrongType;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("composite_type_tests").await;
    let db = ctx.db.get().await?;

    the_attributes_are_the_ones_written(&db).await?;
    a_column_of_the_type_reads_its_attributes(&db).await?;
    a_composite_value_does_not_decode(&db).await?;
    drop_and_rename_reach_a_composite(&db).await?;
    the_server_judges_the_names(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

fn n(name: &str) -> Name {
    Name::runtime(name)
}

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

/// A composite's attributes as the catalogue holds them, in order: name,
/// type as `format_type` spells it, and collation name or `None`.
async fn attributes(
    db: &DatabaseConnection,
    type_name: &str,
) -> Result<Vec<(String, String, Option<String>)>, Error> {
    let rows = db
        .query_all(
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), \
             CASE WHEN a.attcollation <> t.typcollation THEN k.collname::text END \
             FROM pg_type c JOIN pg_attribute a ON a.attrelid = c.typrelid \
             JOIN pg_type t ON t.oid = a.atttypid \
             LEFT JOIN pg_collation k ON k.oid = a.attcollation \
             WHERE c.typname = $1 AND c.typtype = 'c' AND a.attnum > 0 ORDER BY a.attnum",
            &[&type_name],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect())
}

/// Every attribute reaches the catalogue in the order written, with its type
/// as a column would have it — a modifier, an array, an enumeration — and its
/// collation; the empty composite is a type too.
// [spec:pgorm:req:sql.ddl.type-composite+2/test]
async fn the_attributes_are_the_ones_written(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(&Type::create(n("mood")).values(["ok", "bad"]).to_string())
        .await?;
    let create = Type::create(n("address"))
        .attribute_collated(n("Street"), ColumnType::Text, n("C"))
        .attribute(n("no"), ColumnType::Integer)
        .attribute(n("area"), ColumnType::Decimal(Some((10, 2))))
        .attribute(
            n("tags"),
            ColumnType::Array(std::sync::Arc::new(ColumnType::Text)),
        )
        .attribute(n("m"), ColumnType::named("mood"))
        .to_string();
    db.batch_execute(&create).await?;
    let c = || Some("C".to_owned());
    assert_eq!(
        attributes(db, "address").await?,
        [
            ("Street".to_owned(), "text".to_owned(), c()),
            ("no".to_owned(), "integer".to_owned(), None),
            ("area".to_owned(), "numeric(10,2)".to_owned(), None),
            ("tags".to_owned(), "text[]".to_owned(), None),
            ("m".to_owned(), "mood".to_owned(), None),
        ]
    );

    db.batch_execute(&Type::create(n("nothing")).as_composite().to_string())
        .await?;
    assert!(attributes(db, "nothing").await?.is_empty());
    let row = db
        .query_one(
            "SELECT count(*) FROM pg_type WHERE typname = 'nothing' AND typtype = 'c'",
            &[],
        )
        .await?;
    assert_eq!(row.get::<_, i64>(0), 1);

    Ok(())
}

/// A table column declared with the type stores a row of it, and the row's
/// attributes read back by name.
// [spec:pgorm:req:sql.ddl.type-composite+2/test]
async fn a_column_of_the_type_reads_its_attributes(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(
        &Table::create(n("home"))
            .col(ColumnDef::new(n("id")).integer())
            .col(ColumnDef::new_with_type(
                n("addr"),
                ColumnType::named("address"),
            ))
            .to_string(),
    )
    .await?;
    db.batch_execute(
        "INSERT INTO home VALUES \
             (1, ROW('b', 5, 1.5, ARRAY['x'], 'ok')), \
             (2, ROW('B', 6, 2.25, ARRAY[]::text[], 'bad'))",
    )
    .await?;
    let rows: Vec<(String, i32, String)> = db
        .query_all(
            r#"SELECT (addr)."Street", (addr).no, (addr).m::text FROM home ORDER BY id"#,
            &[],
        )
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();
    assert_eq!(
        rows,
        [
            ("b".to_owned(), 5, "ok".to_owned()),
            ("B".to_owned(), 6, "bad".to_owned()),
        ]
    );

    Ok(())
}

/// What the rule says does not read a composite: a decode of the whole value
/// into a `String` model field is refused by the driver, while its text cast
/// reads.
// [spec:pgorm:req:sql.ddl.type-composite+2/test]
async fn a_composite_value_does_not_decode(db: &DatabaseConnection) -> Result<(), Error> {
    let whole = SelectorRaw::<SelectGetableTuple<String>>::into_tuple(
        "SELECT addr FROM home WHERE id = 1",
        Values(vec![]),
    )
    .one(db)
    .await
    .expect_err("a composite value decoded as a String");
    let refused_as_wrong_type = match &whole {
        Error::Postgres(e) => e
            .source()
            .and_then(|cause| cause.downcast_ref::<WrongType>())
            .is_some(),
        _ => false,
    };
    assert!(
        refused_as_wrong_type,
        "the refusal is not the driver's type check: {whole:?}"
    );

    let text = SelectorRaw::<SelectGetableTuple<String>>::into_tuple(
        "SELECT addr::text FROM home WHERE id = 1",
        Values(vec![]),
    )
    .one(db)
    .await?;
    assert_eq!(text, "(b,5,1.50,{x},ok)");

    Ok(())
}

/// The enum path's `DROP TYPE` and `RENAME TO` reach a composite unchanged:
/// a drop is refused while a column has the type (`2BP01`) and `CASCADE`
/// takes the column with it. The label alterations are an enumeration's, and
/// the server refuses them here (`42809`).
// [spec:pgorm:req:sql.ddl.type-composite+2/test]
// [spec:pgorm:req:sql.ddl.type-alter-drop+7/test]
async fn drop_and_rename_reach_a_composite(db: &DatabaseConnection) -> Result<(), Error> {
    let label = db
        .batch_execute(&Type::alter(n("address")).add_value("x").to_string())
        .await
        .expect_err("a composite took an enum label");
    refused_with(&label, &SqlState::WRONG_OBJECT_TYPE);

    db.batch_execute(&Type::alter(n("address")).rename_to(n("place")).to_string())
        .await?;
    assert_eq!(attributes(db, "place").await?.len(), 5);

    let restricted = db
        .batch_execute(&Type::drop(n("place")).restrict().to_string())
        .await
        .expect_err("a composite a column has was dropped");
    refused_with(&restricted, &SqlState::DEPENDENT_OBJECTS_STILL_EXIST);

    db.batch_execute(&Type::drop(n("place")).cascade().to_string())
        .await?;
    let columns = db
        .query_one(
            "SELECT count(*) FROM pg_attribute WHERE attrelid = 'home'::regclass \
             AND attnum > 0 AND NOT attisdropped",
            &[],
        )
        .await?;
    assert_eq!(
        columns.get::<_, i64>(0),
        1,
        "CASCADE left the column behind"
    );

    Ok(())
}

/// The server's refusals the type does not try to make: a repeated attribute
/// (`42701`); a name a relation holds (`42P07`), a composite being a relation
/// itself; and the name of a table, whose row type already has it (`42710`).
// [spec:pgorm:req:sql.ddl.type-composite+2/test]
async fn the_server_judges_the_names(db: &DatabaseConnection) -> Result<(), Error> {
    let repeated = db
        .batch_execute(
            &Type::create(n("twice"))
                .attribute(n("a"), ColumnType::Integer)
                .attribute(n("a"), ColumnType::Text)
                .to_string(),
        )
        .await
        .expect_err("an attribute was declared twice");
    refused_with(&repeated, &SqlState::DUPLICATE_COLUMN);

    db.batch_execute("CREATE SEQUENCE counted").await?;
    let relation = db
        .batch_execute(
            &Type::create(n("counted"))
                .attribute(n("a"), ColumnType::Integer)
                .to_string(),
        )
        .await
        .expect_err("a composite took a sequence's name");
    refused_with(&relation, &SqlState::DUPLICATE_TABLE);

    let row_type = db
        .batch_execute(
            &Type::create(n("home"))
                .attribute(n("a"), ColumnType::Integer)
                .to_string(),
        )
        .await
        .expect_err("a composite took a table's name");
    refused_with(&row_type, &SqlState::DUPLICATE_OBJECT);

    Ok(())
}
