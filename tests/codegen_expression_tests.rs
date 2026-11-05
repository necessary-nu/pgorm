#![allow(unused_imports, dead_code)]

//! The entities pgorm-codegen generates from a schema whose columns carry
//! `DEFAULT`s and generation expressions, against a live server.
//!
//! pgorm-codegen's own tests hold the writer to these files and show the
//! expressions their schema builds render as the ones the bridge read. What
//! only a server settles is that they are the schema's expressions: the
//! catalog the generated entities create holds the expression, and the kind
//! of generated column, that the schema they were generated from does.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::{ConnectionTrait, Schema, entity::prelude::*};
use pretty_assertions::assert_eq;

/// The entities pgorm-codegen generates from its expressions schema: the
/// files its own tests hold the writer's output to, so what is run here is
/// what the generator writes.
#[path = "../pgorm-codegen/tests/sql/expressions/formula.rs"]
mod formula;
#[path = "../pgorm-codegen/tests/sql/expressions/stock_code.rs"]
mod stock_code;
#[path = "../pgorm-codegen/tests/sql/expressions/stock_item.rs"]
mod stock_item;

const SCHEMA: &str = include_str!("../pgorm-codegen/tests/sql/expressions/schema.sql");

/// Each column of `table` with the kind of generated column it is (`s`, `v`
/// or nothing) and its `DEFAULT` or generation expression as the server
/// prints it back.
async fn expressions_of(
    db: &DatabaseConnection,
    table: &str,
) -> Result<Vec<(String, String, Option<String>)>, Error> {
    let rows = db
        .query_all(
            "SELECT a.attname::text, a.attgenerated::text, pg_get_expr(d.adbin, d.adrelid) \
             FROM pg_attribute a \
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             WHERE a.attrelid = $1::text::regclass AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum",
            &[&table],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect())
}

/// The schema, run as written, and the schema its generated entities create
/// hold the same expressions, column for column, in the same kinds: every
/// construct the subset reads and every operator nesting whose precedence
/// matters comes back as the server parsed it the first time. A row the
/// entity inserts with nothing set takes the defaults and is computed by the
/// generated columns, a key the server computes included.
// [spec:pgorm:sem:codegen.entity.expressions+3/test]    against a live server: the entities generated
// from a schema create its DEFAULTs and generation expressions, stored and virtual, unchanged
#[pgorm_macros::test]
async fn generated_entities_keep_their_expressions() -> Result<(), Error> {
    let ctx = TestContext::new("codegen_expression_tests").await;
    let db = ctx.db.get().await?;

    db.batch_execute(SCHEMA).await?;
    let tables = ["stock_item", "formula", "stock_code"];
    let mut written = Vec::new();
    for table in tables {
        written.push(expressions_of(&db, table).await?);
    }
    db.batch_execute("DROP TABLE stock_item, formula, stock_code")
        .await?;
    for create in [
        Schema::new().create_table_from_entity(stock_item::Entity)?,
        Schema::new().create_table_from_entity(formula::Entity)?,
        Schema::new().create_table_from_entity(stock_code::Entity)?,
    ] {
        db.batch_execute(&create.to_string()).await?;
    }
    for (table, expected) in tables.into_iter().zip(&written) {
        assert_eq!(&expressions_of(&db, table).await?, expected, "{table}");
    }
    assert!(
        written[0].contains(&(
            "spare".to_owned(),
            "v".to_owned(),
            Some("(((quantity + 1) * 2) % 7)".to_owned())
        )),
        "{:#?}",
        written[0]
    );

    let item = stock_item::ActiveModel {
        ..Default::default()
    }
    .insert(&db)
    .await?;
    assert_eq!(
        (
            item.name.as_str(),
            item.quantity,
            item.reorder_at,
            item.price,
            item.active,
            item.archived,
            item.note.clone(),
            item.tag_count,
            item.label.as_str(),
        ),
        (
            "it's",
            -1,
            3_000_000_000,
            Decimal::new(150, 2),
            true,
            false,
            None,
            0,
            "x"
        )
    );
    assert_eq!(item.code.len(), 32);
    assert_eq!(
        (item.total, item.shown.clone(), item.in_stock, item.spare),
        (
            Some(Decimal::new(-150, 2)),
            Some("IT'S #-1".to_owned()),
            Some(true),
            Some(0)
        )
    );

    let code = stock_code::ActiveModel {
        base: set(3),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    assert_eq!(code, stock_code::Model { base: 3, code: 6 });

    drop(db);
    ctx.delete().await;
    Ok(())
}
