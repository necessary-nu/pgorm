#![allow(unused_imports, dead_code)]

//! A SELECT's WINDOW clause against a live server.
//!
//! The render tests hold each clause to libpg_query's reading of it: which
//! entries it has, in what order, and which base each extends. What they
//! cannot settle is what the server then does with it — that every entry is
//! read, that an extended window really runs over its base's partitions and
//! order — nor that each clause `check_windows` refuses is one the server
//! refuses too, which only the server can say, so both are read here.
//!
//! The weights are powers of two, so a sum names exactly the rows it read.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    Expr, ExtendedWindow, FrameType, Func, Name, Order, OrderedStatement, Query, SelectStatement,
    WindowDefinition, WindowStatement, error::Error as QueryError,
};
use pgorm::{ConnectionTrait, FromQueryResult, SelectModel, Selector, entity::prelude::*};
use tokio_postgres::error::SqlState;

/// Five readings of two kinds, `a` holding ids 1, 3 and 5.
const SEED: &str = "
    CREATE TABLE reading (id int primary key, kind text not null, weight int not null);
    INSERT INTO reading (id, kind, weight) VALUES
        (1, 'a', 1), (2, 'b', 2), (3, 'a', 4), (4, 'b', 8), (5, 'a', 16);
";

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("named_window_tests").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SEED).await?;

    every_declared_window_is_read(&db).await?;
    an_extended_window_runs_over_its_base(&db).await?;
    the_server_refuses_each_refused_clause(&db).await?;
    executors_refuse_before_sending(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

fn n(name: &str) -> Name {
    Name::runtime(name)
}

fn col(name: &str) -> Expr {
    Expr::col(n(name))
}

/// `SELECT <items> FROM reading … ORDER BY id`, the projection naming
/// windows the caller declares.
fn reading() -> SelectStatement {
    Query::select()
        .from(n("reading"))
        .order_by(n("id"), Order::Asc)
        .take()
}

fn by_kind() -> WindowStatement {
    WindowStatement::partition_by(n("kind"))
}

#[derive(Debug, PartialEq, FromQueryResult)]
struct Totals {
    kind_total: i64,
    everyone: i64,
}

/// Two named windows, each read by its own projection, through the ORM's
/// executor.
// [spec:pgorm:def:sql.ast.window-statement+8/test]    against a live server: the WINDOW clause
// holds every named window, and each is read
async fn every_declared_window_is_read(db: &DatabaseConnection) -> Result<(), Error> {
    let query = reading()
        .expr_window_name_as(Func::sum(col("weight")), n("by_kind"), n("kind_total"))
        .expr_window_name_as(Func::count(col("id")), n("everyone"), n("everyone"))
        .window(n("by_kind"), by_kind())
        .window(n("everyone"), WindowStatement::new())
        .take();
    let rows = Selector::<SelectModel<Totals>>::from_select(query)
        .all(db)
        .await?;
    let total = |kind_total| Totals {
        kind_total,
        everyone: 5,
    };
    assert_eq!(
        rows,
        [total(21), total(10), total(21), total(10), total(21)]
    );
    Ok(())
}

#[derive(Debug, PartialEq, FromQueryResult)]
struct Running {
    running: i64,
    pair: i64,
    place: i64,
}

/// `running` extends `by_kind` with an ORDER BY, and `pair` extends
/// `running` with a frame: each sums within its kind in id order, `pair`
/// over the row before and the row itself. The frame's offset is bound.
// [spec:pgorm:def:sql.ast.window-statement+8/test]    against a live server: an extended window
// copies its base's PARTITION BY and ORDER BY and adds what the base left open
// [spec:pgorm:req:sql.render.window+7/test]    `( "base" ORDER BY … <frame> )`, its offset bound
async fn an_extended_window_runs_over_its_base(db: &DatabaseConnection) -> Result<(), Error> {
    let query = reading()
        .expr_window_name_as(Func::sum(col("weight")), n("running"), n("running"))
        .expr_window_name_as(Func::sum(col("weight")), n("pair"), n("pair"))
        .expr_window_name_as(Func::named(n("row_number")), n("running"), n("place"))
        .window(n("by_kind"), by_kind())
        .window(
            n("running"),
            ExtendedWindow::new(n("by_kind"))
                .order_by(n("id"), Order::Asc)
                .to_owned(),
        )
        .window(
            n("pair"),
            ExtendedWindow::new(n("running"))
                .frame(FrameType::Rows.preceding(1).and_current_row())
                .to_owned(),
        )
        .take();
    let (sql, _) = query.build();
    assert!(
        sql.contains(r#""pair" AS ( "running" ROWS BETWEEN $1 PRECEDING AND CURRENT ROW )"#),
        "{sql}"
    );
    let rows = Selector::<SelectModel<Running>>::from_select(query)
        .all(db)
        .await?;
    let row = |running, pair, place| Running {
        running,
        pair,
        place,
    };
    assert_eq!(
        rows,
        [
            row(1, 1, 1),
            row(2, 2, 1),
            row(1 + 4, 1 + 4, 2),
            row(2 + 8, 2 + 8, 2),
            row(1 + 4 + 16, 4 + 16, 3),
        ]
    );
    Ok(())
}

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

/// A statement reading `OVER "w"`, with `windows` declared.
fn declaring(windows: Vec<(&str, WindowDefinition)>) -> SelectStatement {
    let mut query = reading();
    query.expr_window_name(Func::count(col("id")), n("w"));
    for (name, window) in windows {
        query.window(n(name), window);
    }
    query
}

fn ordered() -> WindowDefinition {
    WindowStatement::new()
        .order_by(n("id"), Order::Asc)
        .take()
        .into()
}

fn framed() -> WindowDefinition {
    WindowStatement::new()
        .frame(FrameType::Rows.current_row())
        .take()
        .into()
}

fn extending(base: &str) -> ExtendedWindow {
    ExtendedWindow::new(n(base))
}

/// Each clause `check_windows` refuses, sent as written, is refused by the
/// server with the SQLSTATE its refusal names; the clauses it passes, the
/// server runs.
// [spec:pgorm:def:sql.ast.window-statement+8/test]    against a live server: `check_windows`
// refuses exactly what the server refuses, with the same codes
async fn the_server_refuses_each_refused_clause(db: &DatabaseConnection) -> Result<(), Error> {
    let cases: Vec<(Vec<(&str, WindowDefinition)>, SqlState)> = vec![
        (
            vec![("w", by_kind().into()), ("w", ordered())],
            SqlState::WINDOWING_ERROR,
        ),
        (vec![("v", by_kind().into())], SqlState::UNDEFINED_OBJECT),
        (
            vec![("w", extending("v").into()), ("v", by_kind().into())],
            SqlState::UNDEFINED_OBJECT,
        ),
        (
            vec![("v", framed()), ("w", extending("v").into())],
            SqlState::WINDOWING_ERROR,
        ),
        (
            vec![
                ("v", ordered()),
                (
                    "w",
                    extending("v")
                        .order_by(n("id"), Order::Desc)
                        .to_owned()
                        .into(),
                ),
            ],
            SqlState::WINDOWING_ERROR,
        ),
        (
            vec![
                ("u", ordered()),
                ("v", extending("u").into()),
                (
                    "w",
                    extending("v")
                        .order_by(n("id"), Order::Desc)
                        .to_owned()
                        .into(),
                ),
            ],
            SqlState::WINDOWING_ERROR,
        ),
    ];
    for (windows, state) in cases {
        let query = declaring(windows);
        let refusal = query.check_windows().expect_err("a refused clause");
        let error = db
            .query_all(query.to_string().as_str(), &[])
            .await
            .expect_err(&format!(
                "the server refuses what is refused as {refusal:?}"
            ));
        refused_with(&error, &state);
    }

    for windows in [
        vec![
            ("u", by_kind().into()),
            (
                "w",
                extending("u")
                    .order_by(n("id"), Order::Asc)
                    .to_owned()
                    .into(),
            ),
        ],
        vec![
            ("u", ordered()),
            (
                "w",
                extending("u")
                    .frame(FrameType::Rows.current_row())
                    .to_owned()
                    .into(),
            ),
        ],
        vec![
            ("u", ordered()),
            ("v", extending("u").into()),
            ("w", extending("v").into()),
        ],
    ] {
        let query = declaring(windows);
        assert_eq!(query.check_windows(), Ok(()));
        db.query_all(query.to_string().as_str(), &[]).await?;
    }
    Ok(())
}

/// pgorm's executor checks the clause before it renders: a refused one comes
/// back as the query builder's error and sends nothing.
// [spec:pgorm:def:sql.ast.window-statement+8/test]    every pgorm executor calls `check_windows`
// before it renders
async fn executors_refuse_before_sending(db: &DatabaseConnection) -> Result<(), Error> {
    let query = declaring(vec![("w", by_kind().into()), ("w", ordered())]);
    let error = Selector::<SelectModel<Totals>>::from_select(query)
        .all(db)
        .await
        .expect_err("two windows of one name");
    assert!(
        matches!(
            &error,
            Error::QueryBuilder(QueryError::WindowRedefined { name }) if name == "w"
        ),
        "{error:?}"
    );
    Ok(())
}
