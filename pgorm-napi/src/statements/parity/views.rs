//! The schema family's view cases — views, a recursive view, a materialized
//! view, its refresh and drop, and `CREATE TABLE AS` — built with
//! pgorm-query's builders directly. Each body's values render inlined.

use pgorm::pgorm_query::{
    BinOper, Expr, ExprTrait, FromItem, MaterializedView, NamedTable, Query, SelectStatement,
    SimpleExpr, Table, TableName, UnionType, Values, View, ViewCheckOption,
};

use super::n;

macro_rules! col {
    ($name:expr) => {
        SimpleExpr::from(Expr::col(n($name)))
    };
}

fn booking() -> FromItem {
    FromItem::Table(NamedTable::from(TableName::SchemaTable(
        n("app"),
        n("booking"),
    )))
}

fn ddl(statement: impl ToString) -> (String, Values) {
    (statement.to_string(), Values(Vec::new()))
}

/// The bookings of a room above 10 whose note is not `it's`: a body with a
/// number and a string, each written as a literal.
fn large_rooms() -> SelectStatement {
    Query::select()
        .expr(col!("id"))
        .expr(col!("room"))
        .from(booking())
        .cond_where(col!("room").binary(BinOper::GreaterThan, 10i64))
        .cond_where(col!("note").binary(BinOper::NotEqual, "it's"))
        .to_owned()
}

fn counting() -> SelectStatement {
    Query::select()
        .expr_as(col!("id"), n("n"))
        .from(booking())
        .union(
            UnionType::All,
            Query::select()
                .expr(col!("n").binary(BinOper::Add, 1i64))
                .from(FromItem::Table(NamedTable::from(TableName::Table(n(
                    "counter",
                )))))
                .cond_where(col!("n").binary(BinOper::SmallerThan, 5i64))
                .to_owned(),
        )
        .to_owned()
}

/// Why pgorm-query's constructors take every body here: none has a tie limit
/// or a window for the definition check to refuse.
const BODY_TAKEN: &str = "the body has nothing the definition check refuses";

fn app(name: &str) -> TableName {
    TableName::SchemaTable(n("app"), n(name))
}

/// Every view case, named as the golden file names it.
pub(super) fn cases() -> Vec<(&'static str, (String, Values))> {
    vec![
        (
            "create-view",
            ddl(View::create(app("large_room"), large_rooms()).expect(BODY_TAKEN)),
        ),
        (
            "create-view-options",
            ddl(View::create(app("large_room"), large_rooms())
                .expect(BODY_TAKEN)
                .or_replace()
                .columns([n("booking"), n("room_no")])
                .check_option(ViewCheckOption::Local)
                .check_option(ViewCheckOption::Cascaded)
                .security_barrier()
                .security_invoker()
                .to_owned()),
        ),
        (
            "create-recursive-view",
            ddl(
                View::create_recursive(TableName::Table(n("counter")), n("n"), counting())
                    .expect(BODY_TAKEN)
                    .or_replace()
                    .security_barrier()
                    .to_owned(),
            ),
        ),
        (
            "drop-views",
            ddl(View::drop(TableName::Table(n("large_room")))
                .name(app("other"))
                .if_exists()
                .cascade()
                .to_owned()),
        ),
        (
            "create-materialized-view",
            ddl(MaterializedView::create(app("room_total"), large_rooms())
                .expect(BODY_TAKEN)
                .if_not_exists()
                .columns([n("booking"), n("room_no")])
                .with_no_data()
                .to_owned()),
        ),
        (
            "refresh-materialized-view",
            ddl(MaterializedView::refresh(app("room_total"))),
        ),
        (
            "refresh-materialized-view-concurrently",
            ddl(MaterializedView::refresh(app("room_total"))
                .concurrently()
                .to_owned()),
        ),
        (
            "refresh-materialized-view-no-data",
            ddl(MaterializedView::refresh(app("room_total"))
                .with_no_data()
                .to_owned()),
        ),
        (
            "drop-materialized-views",
            ddl(MaterializedView::drop(app("room_total"))
                .name(TableName::Table(n("other")))
                .restrict()
                .to_owned()),
        ),
        (
            "create-table-as",
            ddl(
                Table::create_as(TableName::Table(n("large_copy")), large_rooms())
                    .expect(BODY_TAKEN)
                    .if_not_exists()
                    .columns([n("booking")])
                    .with_no_data()
                    .to_owned(),
            ),
        ),
    ]
}
