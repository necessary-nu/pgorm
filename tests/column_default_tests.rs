#![allow(unused_imports, dead_code)]

//! A column's `DEFAULT` against a live server.
//!
//! A column definition reads its default as a `b_expr`, the grammar's narrow
//! operand with no logical operator, `IS`, `BETWEEN`, `IN` or `LIKE`, so an
//! operator expression there is parenthesised; `ALTER COLUMN .. SET DEFAULT`
//! reads the full expression and takes one bare. Each default here is read
//! back from a row written without the column, so a default the server
//! parsed as something else would show in the value.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{ColumnDef, ColumnType, Expr, Func, Name, SimpleExpr, Table};
use pgorm::{ConnectionTrait, Error};

fn n(name: &str) -> Name {
    Name::runtime(name)
}

fn both() -> SimpleExpr {
    SimpleExpr::from(Expr::val(true)).and(Expr::val(false).into())
}

// [spec:pgorm:req:sql.ddl.column-def+13/test]    against a live server: an operator expression
// is a column's DEFAULT in CREATE TABLE and ADD COLUMN, and SET DEFAULT takes one bare
#[pgorm_macros::test]
async fn operator_defaults_reach_the_server() -> Result<(), Error> {
    let ctx = TestContext::new("operator_defaults_reach_the_server").await;
    let db = ctx.db.get().await?;

    let create = Table::create(n("flag"))
        .col(ColumnDef::new(n("id")).integer())
        .col(ColumnDef::new(n("set")).boolean().default(both().not()))
        .col(
            ColumnDef::new(n("sum"))
                .integer()
                .default(SimpleExpr::from(Expr::val(1)).add(2).mul(3)),
        )
        .to_string();
    db.batch_execute(&create).await?;
    let add = Table::alter(n("flag"))
        .add_column(
            ColumnDef::new(n("between"))
                .boolean()
                .default(Expr::val(2).between(1, 3)),
        )
        .to_string();
    db.batch_execute(&add).await?;
    db.batch_execute("INSERT INTO flag (id) VALUES (1)").await?;

    let set = Table::alter(n("flag"))
        .modify_column(ColumnDef::new(n("set")).default(both()))
        .to_string();
    assert!(set.ends_with(r#"SET DEFAULT TRUE AND FALSE"#), "{set}");
    db.batch_execute(&set).await?;
    db.batch_execute("INSERT INTO flag (id) VALUES (2)").await?;

    let rows = db
        .query_all(
            r#"SELECT id, "set", "sum", "between" FROM flag ORDER BY id"#,
            &[],
        )
        .await?;
    let read: Vec<(i32, bool, i32, bool)> = rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
        .collect();
    assert_eq!(read, [(1, true, 9, true), (2, false, 9, true)]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The SQLSTATE a refusal carries.
fn state(error: &Error) -> Option<String> {
    match error {
        Error::Postgres(error) => error.code().map(|code| code.code().to_owned()),
        _ => None,
    }
}

/// A column of one enum retyped to another, which no assignment cast joins:
/// `modify_column`'s retype is refused (`42804`), and so is the `USING`
/// retype while the column's default cannot be cast (`42804`); with the
/// default dropped, the `USING` expression converts each row through its
/// text, and the column holds the new type with no default. A generated
/// column takes no `USING` (`42611`), and dropping a default a column lacks
/// changes nothing.
// [spec:pgorm:req:sql.ddl.alter-table+15/test]    against a live server: ALTER COLUMN .. TYPE
// .. USING converts each row, refused where a default or a generated column stands in its way,
// and DROP DEFAULT clears the way
#[pgorm_macros::test]
async fn a_column_is_retyped_using_an_expression() -> Result<(), Error> {
    let ctx = TestContext::new("column_retyped_using").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        "CREATE TYPE grade AS ENUM ('low', 'high'); \
         CREATE TYPE grade_next AS ENUM ('low', 'mid', 'high'); \
         CREATE TABLE sample (id integer, grade grade NOT NULL DEFAULT 'low', \
         fixed grade GENERATED ALWAYS AS ('high') STORED); \
         INSERT INTO sample (id, grade) VALUES (1, 'high'), (2, 'low')",
    )
    .await?;
    let alter = || Table::alter(n("sample"));
    let next = || pgorm::pgorm_query::ColumnType::named("grade_next");
    let through_text = |column: &str| {
        Expr::col(n(column))
            .cast_as(n("text"))
            .cast_as(n("grade_next"))
    };

    let assigned = db
        .batch_execute(
            &alter()
                .modify_column(ColumnDef::new(n("grade")).named(next_name()))
                .to_string(),
        )
        .await
        .expect_err("no assignment cast joins two enums");
    assert_eq!(state(&assigned).as_deref(), Some("42804"), "{assigned}");

    let using = alter()
        .alter_column_type_using(n("grade"), next(), through_text("grade"))
        .to_string();
    assert_eq!(
        using,
        r#"ALTER TABLE "sample" ALTER COLUMN "grade" TYPE grade_next USING CAST(CAST("grade" AS text) AS grade_next)"#
    );
    let defaulted = db
        .batch_execute(&using)
        .await
        .expect_err("the default cannot be cast");
    assert_eq!(state(&defaulted).as_deref(), Some("42804"), "{defaulted}");

    db.batch_execute(
        &alter()
            .drop_default(n("grade"))
            .alter_column_type_using(n("grade"), next(), through_text("grade"))
            .to_string(),
    )
    .await?;
    let rows: Vec<(i32, String)> = db
        .query_all("SELECT id, grade::text FROM sample ORDER BY id", &[])
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(rows, [(1, "high".to_owned()), (2, "low".to_owned())]);
    let (type_name, default): (String, Option<String>) = db
        .query_one(
            "SELECT format_type(atttypid, atttypmod), pg_get_expr(adbin, adrelid) \
             FROM pg_attribute LEFT JOIN pg_attrdef ON adrelid = attrelid AND adnum = attnum \
             WHERE attrelid = 'sample'::regclass AND attname = 'grade'",
            &[],
        )
        .await
        .map(|row| (row.get(0), row.get(1)))?;
    assert_eq!((type_name.as_str(), default), ("grade_next", None));
    db.batch_execute(&alter().drop_default(n("grade")).to_string())
        .await?;

    let generated = db
        .batch_execute(
            &alter()
                .alter_column_type_using(n("fixed"), next(), through_text("fixed"))
                .to_string(),
        )
        .await
        .expect_err("a generated column takes no USING");
    assert_eq!(state(&generated).as_deref(), Some("42611"), "{generated}");

    drop(db);
    ctx.delete().await;
    Ok(())
}

fn next_name() -> pgorm::pgorm_query::TypeName {
    pgorm::pgorm_query::TypeName::new(n("grade_next"))
}

/// A column retyped under a collation: each row converted through the
/// `USING` expression and the column holding the collation named; a
/// collation on a type that takes none is refused (`42804`), and so is one the
/// server does not have (`42704`).
// [spec:pgorm:req:sql.ddl.alter-table+15/test]    against a live server: ALTER COLUMN .. TYPE
// .. COLLATE .. USING converts each row and gives the column the collation, refused on a type
// without one and for a collation the server lacks
#[pgorm_macros::test]
async fn a_column_is_retyped_under_a_collation() -> Result<(), Error> {
    let ctx = TestContext::new("column_retyped_collated").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        "CREATE TABLE sample (id integer, label text, amount integer); \
         INSERT INTO sample VALUES (1, 'b', 1), (2, 'a', 2)",
    )
    .await?;
    let alter = || Table::alter(n("sample"));

    db.batch_execute(
        &alter()
            .alter_column_type_collated_using(
                n("label"),
                ColumnType::string(Some(8)),
                (n("pg_catalog"), n("C")),
                Func::upper(Expr::col(n("label"))),
            )
            .to_string(),
    )
    .await?;
    let rows: Vec<(String, String, String)> = db
        .query_all(
            "SELECT s.label::text, format_type(a.atttypid, a.atttypmod), k.collname::text \
             FROM sample s, pg_attribute a JOIN pg_collation k ON k.oid = a.attcollation \
             WHERE a.attrelid = 'sample'::regclass AND a.attname = 'label' ORDER BY s.id",
            &[],
        )
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();
    let row = |label: &str| {
        (
            label.to_owned(),
            "character varying(8)".to_owned(),
            "C".to_owned(),
        )
    };
    assert_eq!(rows, [row("B"), row("A")]);

    let uncollatable = db
        .batch_execute(
            &alter()
                .alter_column_type_collated_using(
                    n("amount"),
                    ColumnType::BigInteger,
                    n("C"),
                    Expr::col(n("amount")).mul(10),
                )
                .to_string(),
        )
        .await
        .expect_err("bigint takes no collation");
    assert_eq!(
        state(&uncollatable).as_deref(),
        Some("42804"),
        "{uncollatable}"
    );
    let missing = db
        .batch_execute(
            &alter()
                .alter_column_type_collated_using(
                    n("label"),
                    ColumnType::Text,
                    n("no such collation"),
                    Expr::col(n("label")),
                )
                .to_string(),
        )
        .await
        .expect_err("the server has no such collation");
    assert_eq!(state(&missing).as_deref(), Some("42704"), "{missing}");

    drop(db);
    ctx.delete().await;
    Ok(())
}
