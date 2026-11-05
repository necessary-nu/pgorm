#![allow(unused_imports, dead_code)]

//! `COLLATE` against a live PostgreSQL server.
//!
//! The render tests in pgorm-query hold each spelling to libpg_query's
//! `CollateClause`. What only a server settles is what the clause *does*: the
//! order the same values sort in, which strings compare equal, and that a
//! column declared with a collation compares by it without a statement saying
//! so. Each case below sets the collated answer beside the one the same
//! values give under another collation, so a clause that rendered and was
//! then ignored still fails.
//!
//! The server's collations are read off `pg_collation` rather than assumed:
//! `"C"` exists everywhere, a linguistic collation is whichever the server
//! has, ICU's root locale preferred, and the nondeterministic case runs only
//! where ICU does.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    ColumnDef, Expr, Func, Name, Order, OrderedStatement, Query, SimpleExpr, Table,
};
use pgorm::{ConnectionTrait, SelectGetableTuple, SelectorRaw, entity::prelude::*};
use tokio_postgres::error::SqlState;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("collation_tests").await;
    let db = ctx.db.get().await?;

    db.batch_execute(
        "CREATE TABLE word (w text NOT NULL); \
         INSERT INTO word (w) VALUES ('b'), ('A'), ('a'), ('B')",
    )
    .await?;
    let linguistic = linguistic_collation(&db).await?;

    the_same_values_sort_by_the_collation_named(&db, &linguistic).await?;
    a_nondeterministic_collation_equates_distinct_bytes(&db).await?;
    a_collated_column_compares_by_its_collation(&db, &linguistic).await?;
    an_unknown_collation_is_refused(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

/// A collation that orders text by language rather than by byte, from what
/// this server has: ICU's root locale when ICU is built in, otherwise any
/// libc locale that is not `C` or `POSIX` under another name.
async fn linguistic_collation(db: &DatabaseConnection) -> Result<String, Error> {
    let row = db
        .query_one(
            "SELECT collname FROM pg_collation \
             WHERE collencoding IN (-1, (SELECT encoding FROM pg_database \
                                         WHERE datname = current_database())) \
               AND (collname = 'und-x-icu' \
                    OR (collprovider = 'c' AND collcollate NOT IN ('C', 'POSIX') \
                        AND collcollate NOT LIKE 'C.%')) \
             ORDER BY (collname = 'und-x-icu') DESC, collname \
             LIMIT 1",
            &[],
        )
        .await?;
    Ok(row.get(0))
}

/// The words, in the order `order` sorts them.
async fn sorted(db: &DatabaseConnection, order: SimpleExpr) -> Result<Vec<String>, Error> {
    let query = Query::select()
        .column(Name::runtime("w"))
        .from(Name::runtime("word"))
        .order_by_expr(order, Order::Asc)
        .to_string();
    let rows = db.query_all(&query, &[]).await?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

fn word() -> Expr {
    Expr::col(Name::runtime("w"))
}

/// Under `"C"` text sorts by byte, so every upper-case letter comes before
/// every lower-case one; a linguistic collation puts both spellings of `a`
/// before both spellings of `b`. Same rows, same statement shape, two orders.
// [spec:pgorm:req:sql.ast.expr.collate/test]    against a live server: ORDER BY sorts by the
// collation its key names
// [spec:pgorm:req:sql.render.collate/test]
// [spec:pgorm:req:sql.scope+23/test]
async fn the_same_values_sort_by_the_collation_named(
    db: &DatabaseConnection,
    linguistic: &str,
) -> Result<(), Error> {
    let by_byte = sorted(db, word().collate(Name::runtime("C")).into()).await?;
    assert_eq!(by_byte, ["A", "B", "a", "b"]);

    let by_language = sorted(db, word().collate(Name::runtime(linguistic)).into()).await?;
    let first_b = by_language
        .iter()
        .position(|w| w.eq_ignore_ascii_case("b"))
        .expect("a b");
    assert_eq!(first_b, 2, "{linguistic} sorted {by_language:?}");

    // The comparison operators answer by the same order: `'a' < 'B'` is false
    // byte-wise and true by language. Both operands are bound, and the
    // collated placeholder is typed `text` by the clause itself.
    let less = |collation: &str| {
        Query::select()
            .expr(Expr::val("a").collate(Name::runtime(collation)).lt("B"))
            .build()
    };
    for (collation, expected) in [("C", false), (linguistic, true)] {
        let (sql, values) = less(collation);
        assert!(sql.starts_with("SELECT ($1 COLLATE "), "{sql}");
        let answer = SelectorRaw::<SelectGetableTuple<bool>>::into_tuple::<bool>(sql, values)
            .one(db)
            .await?;
        assert_eq!(answer, expected, "{collation}");
    }

    Ok(())
}

/// A nondeterministic collation compares strings that differ in bytes as
/// equal: under ICU's secondary strength `a` and `A` are one string, so an
/// equality finds both rows where the control finds one. Without ICU the
/// server cannot create one, and says so.
// [spec:pgorm:req:sql.ast.expr.collate/test]    against a live server: equality answers by a
// nondeterministic collation
// [spec:pgorm:req:sql.scope+23/test]
async fn a_nondeterministic_collation_equates_distinct_bytes(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let create = "CREATE COLLATION case_insensitive \
                  (provider = icu, locale = 'und-u-ks-level2', deterministic = false)";
    let icu = db
        .query_one(
            "SELECT count(*) FROM pg_collation WHERE collprovider = 'i'",
            &[],
        )
        .await?
        .get::<_, i64>(0)
        > 0;
    if !icu {
        let refused = db.batch_execute(create).await.expect_err("no ICU");
        refused_with(&refused, &SqlState::FEATURE_NOT_SUPPORTED);
        return Ok(());
    }
    db.batch_execute(create).await?;

    let count = |equal: SimpleExpr| {
        Query::select()
            .expr(Func::count(word()))
            .from(Name::runtime("word"))
            .and_where(equal)
            .to_string()
    };
    let control = db.query_one(&count(word().eq("a")), &[]).await?;
    assert_eq!(control.get::<_, i64>(0), 1);
    let folded = count(word().collate(Name::runtime("case_insensitive")).eq("a"));
    let row = db.query_one(&folded, &[]).await?;
    assert_eq!(row.get::<_, i64>(0), 2, "{folded}");

    Ok(())
}

/// A column declared `COLLATE "C"` sorts and compares by it with no clause in
/// the query, beside a column declared with a linguistic collation that holds
/// the same values; retyping the second through `ALTER TABLE` moves it to
/// `"C"` as well.
// [spec:pgorm:req:sql.ddl.column-def+13/test]    against a live server: a column compares by the
// collation it was declared with
// [spec:pgorm:req:sql.ddl.alter-table+15/test]    the retype carries the collation
// [spec:pgorm:req:sql.scope+23/test]
async fn a_collated_column_compares_by_its_collation(
    db: &DatabaseConnection,
    linguistic: &str,
) -> Result<(), Error> {
    let create = Table::create(Name::runtime("label"))
        .col(
            ColumnDef::new(Name::runtime("by_byte"))
                .text()
                .collate(Name::runtime("C"))
                .not_null(),
        )
        .col(
            ColumnDef::new(Name::runtime("by_language"))
                .text()
                .collate(Name::runtime(linguistic))
                .not_null(),
        )
        .to_string();
    db.batch_execute(&create).await?;
    db.batch_execute("INSERT INTO label VALUES ('a', 'a'), ('B', 'B')")
        .await?;

    let first = |column: &'static str| {
        Query::select()
            .column(Name::runtime(column))
            .from(Name::runtime("label"))
            .order_by(Name::runtime(column), Order::Asc)
            .limit(1)
            .to_string()
    };
    let row = db.query_one(&first("by_byte"), &[]).await?;
    assert_eq!(row.get::<_, String>(0), "B");
    let row = db.query_one(&first("by_language"), &[]).await?;
    assert_eq!(row.get::<_, String>(0), "a", "{linguistic}");

    let declared = db
        .query_one(
            "SELECT k.collname FROM pg_attribute a \
             JOIN pg_collation k ON k.oid = a.attcollation \
             WHERE a.attrelid = 'label'::regclass AND a.attname = 'by_byte'",
            &[],
        )
        .await?;
    assert_eq!(declared.get::<_, String>(0), "C");

    let retype = Table::alter(Name::runtime("label"))
        .modify_column(
            ColumnDef::new(Name::runtime("by_language"))
                .text()
                .collate(Name::runtime("C")),
        )
        .to_string();
    db.batch_execute(&retype).await?;
    let row = db.query_one(&first("by_language"), &[]).await?;
    assert_eq!(row.get::<_, String>(0), "B", "{retype}");

    Ok(())
}

/// A collation the server does not have is refused as an undefined object
/// (`42704`). The name is quoted, so its case is the caller's: `c` is not
/// `"C"`, as an unquoted `C` would not be either.
// [spec:pgorm:req:sql.ast.expr.collate/test]    against a live server: an unknown collation is
// refused, and the name is matched case-sensitively
// [spec:pgorm:req:sql.scope+23/test]
async fn an_unknown_collation_is_refused(db: &DatabaseConnection) -> Result<(), Error> {
    for name in ["no_such_collation", "c"] {
        let query = Query::select()
            .expr(word().collate(Name::runtime(name)))
            .from(Name::runtime("word"))
            .to_string();
        let refused = db.query_all(&query, &[]).await.expect_err(name);
        refused_with(&refused, &SqlState::UNDEFINED_OBJECT);
    }

    Ok(())
}
