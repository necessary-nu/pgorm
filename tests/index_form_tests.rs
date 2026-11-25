#![allow(unused_imports, dead_code)]

//! Partial, expression, `INCLUDE` and operator-class indexes against a live
//! PostgreSQL server.
//!
//! The render tests in pgorm-query feed the SQL to the real grammar, which
//! settles that it parses. What only a server settles is that these clauses do
//! the work they are reached for: that a partial unique index constrains the
//! rows its predicate accepts and leaves the rest alone, that an expression
//! index is the thing a `lower()` lookup actually reads, that `INCLUDE` adds
//! columns outside the key rather than inside it, and that an operator class
//! rendered as a quoted identifier is one the server resolves. Each is checked
//! against the catalogue or against behaviour, not against the same SQL that
//! produced it.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    ColumnDef, ConditionalStatement, Expr, Func, Index, IndexColumn, IndexOrder, Name, Table,
};
use pgorm::{Connection, entity::prelude::*};

const SEED: &str = "
    CREATE TABLE doc (
        id int PRIMARY KEY,
        owner_id int NOT NULL,
        name text NOT NULL,
        body text,
        active bool NOT NULL DEFAULT false
    );
";

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("index_form_tests").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SEED).await?;

    partial_unique_constrains_only_its_predicate(&db).await?;
    expression_index_answers_a_lower_lookup(&db).await?;
    include_columns_sit_outside_the_key(&db).await?;
    operator_class_resolves_as_a_quoted_name(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

/// The definition PostgreSQL reports for an index, which is its own rendering
/// of what it stored rather than an echo of the statement.
async fn indexdef(db: &DatabaseConnection, name: &str) -> Result<String, Error> {
    Ok(db
        .query_one(
            "SELECT indexdef FROM pg_indexes WHERE indexname = $1",
            &[&name],
        )
        .await?
        .get::<_, String>(0))
}

/// The reason partial indexes exist: one active row per owner, with no
/// constraint at all on the inactive ones.
// [spec:pgorm:req:sql.ddl.index-create+11/test]
async fn partial_unique_constrains_only_its_predicate(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let sql = Index::create(Name::runtime("doc"), Name::runtime("owner_id"))
        .name(Name::runtime("doc_one_active_per_owner"))
        .unique()
        .and_where(Expr::col(Name::runtime("active")).into())
        .to_string();
    assert_eq!(
        sql,
        [
            r#"CREATE UNIQUE INDEX "doc_one_active_per_owner" ON "doc" ("owner_id")"#,
            r#"WHERE "active""#,
        ]
        .join(" ")
    );
    db.batch_execute(&sql).await?;

    assert!(
        indexdef(db, "doc_one_active_per_owner")
            .await?
            .contains("WHERE active"),
        "the server did not store a partial index"
    );

    db.batch_execute(
        "INSERT INTO doc (id, owner_id, name, active) VALUES
            (1, 1, 'alpha', true),
            (2, 1, 'beta', false),
            (3, 1, 'gamma', false);",
    )
    .await?;

    // Inside the predicate: a second active row for the same owner is refused.
    let refused = db
        .execute(
            "INSERT INTO doc (id, owner_id, name, active) VALUES (4, 1, 'delta', true)",
            &[],
        )
        .await;
    assert!(
        refused.is_err(),
        "the partial unique index did not constrain its own rows"
    );

    // Outside it: three inactive rows for one owner are fine, which a whole-table
    // unique index would have refused.
    db.execute(
        "INSERT INTO doc (id, owner_id, name, active) VALUES (5, 1, 'epsilon', false)",
        &[],
    )
    .await?;
    let inactive = db
        .query_one(
            "SELECT count(*) FROM doc WHERE owner_id = 1 AND NOT active",
            &[],
        )
        .await?;
    assert_eq!(inactive.get::<_, i64>(0), 3);

    Ok(())
}

/// An expression index is only worth building if a query reads it, so this
/// checks the plan as well as the answer.
// [spec:pgorm:req:sql.ddl.index-create+11/test]
async fn expression_index_answers_a_lower_lookup(db: &DatabaseConnection) -> Result<(), Error> {
    let sql = Index::create(
        Name::runtime("doc"),
        IndexColumn::expr(Func::lower(Expr::col(Name::runtime("name")))),
    )
    .name(Name::runtime("doc_name_lower"))
    .to_string();
    assert_eq!(
        sql,
        r#"CREATE INDEX "doc_name_lower" ON "doc" ((LOWER("name")))"#
    );
    db.batch_execute(&sql).await?;

    let definition = indexdef(db, "doc_name_lower").await?;
    assert!(
        definition.contains("lower(name)"),
        "unexpected stored definition: {definition}"
    );

    // The lookup returns what it should, spelled the way the index was built.
    let found = db
        .query_one("SELECT id FROM doc WHERE lower(name) = 'alpha'", &[])
        .await?;
    assert_eq!(found.get::<_, i32>(0), 1);

    // And the planner will use it when it is the cheaper option.
    db.batch_execute(
        "INSERT INTO doc (id, owner_id, name)
         SELECT i, 2, 'Name' || i FROM generate_series(100, 2000) AS i;
         ANALYZE doc;",
    )
    .await?;
    let plan: String = db
        .query_all(
            "EXPLAIN SELECT id FROM doc WHERE lower(name) = 'name500'",
            &[],
        )
        .await?
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plan.contains("doc_name_lower"),
        "the planner ignored the expression index:\n{plan}"
    );

    Ok(())
}

/// `INCLUDE` columns are payload, not key: the catalogue counts them
/// separately, and uniqueness ignores them.
// [spec:pgorm:req:sql.ddl.index-create+11/test]
async fn include_columns_sit_outside_the_key(db: &DatabaseConnection) -> Result<(), Error> {
    let sql = Index::create(Name::runtime("doc"), Name::runtime("name"))
        .name(Name::runtime("doc_name_with_body"))
        .unique()
        .include([Name::runtime("body")])
        .to_string();
    assert_eq!(
        sql,
        r#"CREATE UNIQUE INDEX "doc_name_with_body" ON "doc" ("name") INCLUDE ("body")"#
    );
    db.batch_execute(&sql).await?;

    // Two attributes in the index, one of them a key attribute.
    let counts = db
        .query_one(
            "SELECT indnatts, indnkeyatts FROM pg_index
             WHERE indexrelid = 'doc_name_with_body'::regclass",
            &[],
        )
        .await?;
    assert_eq!(
        (counts.get::<_, i16>(0), counts.get::<_, i16>(1)),
        (2, 1),
        "the included column was treated as part of the key"
    );

    Ok(())
}

/// The operator class renders as a quoted identifier; this settles that the
/// server resolves one written that way.
// [spec:pgorm:req:sql.ddl.index-create+11/test]
async fn operator_class_resolves_as_a_quoted_name(db: &DatabaseConnection) -> Result<(), Error> {
    let sql = Index::create(
        Name::runtime("doc"),
        IndexColumn::name(Name::runtime("name"))
            .operator_class(Name::runtime("text_pattern_ops"))
            .order(IndexOrder::Desc),
    )
    .name(Name::runtime("doc_name_pattern"))
    .to_string();
    assert_eq!(
        sql,
        r#"CREATE INDEX "doc_name_pattern" ON "doc" ("name" "text_pattern_ops" DESC)"#
    );
    db.batch_execute(&sql).await?;

    let definition = indexdef(db, "doc_name_pattern").await?;
    assert!(
        definition.contains("text_pattern_ops"),
        "unexpected stored definition: {definition}"
    );

    Ok(())
}
