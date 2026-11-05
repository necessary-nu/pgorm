#![allow(unused_imports, dead_code)]

//! A locking clause's `OF` list against a live server.
//!
//! The list holds unqualified names, so the shapes the grammar refuses there
//! (an aliased table written as its source, a subquery, a function call, a
//! sample, a schema-qualified name) have no construction. What a name then
//! refers to is the server's to say: it is matched against the names the FROM
//! items answer to, and only a relation's rows can be locked. Each accepted
//! lock is observed from a second connection, whose `NOWAIT` read of the
//! locked rows fails while the first holds them.

use pgorm_query::ExprTrait;
pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    Asterisk, CommonTableExpression, Expr, FromItem, Func, IntoFromItem, IntoNamedTable, LockType,
    Name, NamedTable, Query, SampleMethod, SelectStatement, TableSample, WithClause,
};
use pgorm::{
    ConnectionTrait, DatabaseConnection, Error, SelectGetableTuple, Selector, TransactionTrait,
};
use tokio_postgres::error::SqlState;

const SEED: &str = "
    CREATE TABLE stock (id int PRIMARY KEY, qty int NOT NULL);
    INSERT INTO stock (id, qty) VALUES (1, 10), (2, 20);
";

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("row_lock_tests").await;
    let mut db = ctx.db.get().await?;
    let other = ctx.db.get().await?;
    db.batch_execute(SEED).await?;

    of_names_lock_the_rows_they_answer_for(&mut db, &other).await?;
    the_server_refuses_names_it_cannot_lock(&db).await?;

    drop(other);
    drop(db);
    ctx.delete().await;

    Ok(())
}

fn n(name: &str) -> Name {
    Name::runtime(name)
}

fn sql_state(err: &Error) -> Option<&SqlState> {
    match err {
        Error::Postgres(e) => e.code(),
        _ => None,
    }
}

fn aliased(alias: &str) -> NamedTable {
    n("stock").into_named_table().alias(n(alias))
}

/// Run `query` through `build`, its values bound, counting the rows it reads.
async fn count<C: ConnectionTrait>(db: &C, query: SelectStatement) -> Result<usize, Error> {
    Ok(
        Selector::<SelectGetableTuple<i32>>::into_tuple::<i32>(query)
            .all(db)
            .await?
            .len(),
    )
}

fn locking(from: impl IntoFromItem, of: &str) -> SelectStatement {
    Query::select()
        .column(Asterisk)
        .from(from)
        .lock_with_tables(LockType::Update, [n(of)])
        .take()
}

/// Whether `other` finds a row of `stock` locked: its `NOWAIT` read fails
/// with `55P03` exactly when one is.
async fn stock_is_locked(other: &DatabaseConnection) -> Result<bool, Error> {
    match other
        .query_all("SELECT id FROM stock FOR UPDATE NOWAIT", &[])
        .await
    {
        Ok(_) => Ok(false),
        Err(err) if sql_state(&err) == Some(&SqlState::LOCK_NOT_AVAILABLE) => Ok(true),
        Err(err) => Err(err),
    }
}

// [spec:pgorm:def:sql.ast.select+6/test]    against a live server: OF takes the name a FROM
// item answers to, an aliased table's alias, a subquery's and a sample's, and locks its rows
// [spec:pgorm:sem:sql.render.locking+1/test]
async fn of_names_lock_the_rows_they_answer_for(
    db: &mut DatabaseConnection,
    other: &DatabaseConnection,
) -> Result<(), Error> {
    let subquery = FromItem::SubQuery(
        Query::select().column(n("id")).from(n("stock")).take(),
        n("q"),
    );
    let sample = TableSample::new(aliased("s"), SampleMethod::System, 100);
    for statement in [
        locking(n("stock"), "stock"),
        locking(aliased("s"), "s"),
        locking(subquery, "q"),
        locking(sample, "s"),
    ] {
        assert!(!stock_is_locked(other).await?);
        let outcome = db
            .transaction(async |txn| {
                let sql = statement.to_string();
                assert_eq!(count(&txn, statement).await?, 2);
                assert!(stock_is_locked(other).await?, "{sql}");
                roll_back::<()>()
            })
            .await;
        assert_rolled_back(outcome);
    }
    Ok(())
}

// [spec:pgorm:def:sql.ast.select+6/test]    against a live server: a name no FROM item answers
// to, an aliased table's own name among them, is its 42P01, and a function's or a common table
// expression's name its 0A000, nothing there holding rows to lock
async fn the_server_refuses_names_it_cannot_lock(db: &DatabaseConnection) -> Result<(), Error> {
    let refusal = |statement: SelectStatement| async move {
        let err = count(db, statement)
            .await
            .expect_err("the server refuses the lock");
        sql_state(&err).cloned()
    };
    let series = FromItem::FunctionCall(
        Func::named(n("generate_series"))
            .arg(Expr::val(1).cast_as(n("int4")))
            .arg(Expr::val(2).cast_as(n("int4"))),
        n("f"),
    );
    let mut cte = locking(n("c"), "c");
    cte.with(WithClause::new(CommonTableExpression::new(
        n("c"),
        Query::select().column(Asterisk).from(n("stock")).take(),
    )));

    assert_eq!(
        refusal(locking(aliased("s"), "stock")).await,
        Some(SqlState::UNDEFINED_TABLE)
    );
    assert_eq!(
        refusal(locking(n("stock"), "elsewhere")).await,
        Some(SqlState::UNDEFINED_TABLE)
    );
    assert_eq!(
        refusal(locking(series, "f")).await,
        Some(SqlState::FEATURE_NOT_SUPPORTED)
    );
    assert_eq!(refusal(cte).await, Some(SqlState::FEATURE_NOT_SUPPORTED));
    Ok(())
}
