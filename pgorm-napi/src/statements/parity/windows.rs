//! The window and range families' cases, built with pgorm-query directly.

use std::ops::Bound;

use pgorm::pgorm_query::{
    ArrayType, BinOper, Expr, ExprTrait, ExtendedWindow, FrameClause, FrameExclusion, FrameType,
    FromItem, Func, Multirange, NamedTable, Order, OverStatement, Query, Range, RangeType,
    SelectStatement, SimpleExpr, TableName, TypeName, Value, Values, WindowStatement,
};

use super::n;

macro_rules! col {
    ($name:expr) => {
        SimpleExpr::from(Expr::col(n($name)))
    };
}

macro_rules! at {
    ($name:expr) => {
        SimpleExpr::from(Expr::col((n("r"), n($name))))
    };
}

macro_rules! table {
    ($name:expr) => {
        FromItem::Table(NamedTable::from(TableName::Table(n($name))))
    };
}

macro_rules! reading {
    () => {
        FromItem::Table(NamedTable::from(TableName::Table(n("reading"))).alias(n("r")))
    };
}

macro_rules! named {
    ($name:expr, $args:expr) => {
        Func::named(n($name)).args($args)
    };
}

fn inline_and_named() -> (String, Values) {
    let mut running = WindowStatement::default();
    running.add_partition_by(at!("kind"));
    running.order_by_expr(at!("at"), Order::Asc);
    let mut by_kind = WindowStatement::default();
    by_kind.add_partition_by(at!("kind"));
    Query::select()
        .expr(at!("id"))
        .expr_window_as(Func::sum(at!("weight")), running, n("running"))
        .expr_window_name_as(
            named!("row_number", Vec::<SimpleExpr>::new()),
            n("by_kind"),
            n("n"),
        )
        .expr_window(Func::avg(at!("weight")), WindowStatement::default())
        .from(reading!())
        .window(n("by_kind"), by_kind)
        .build()
}

fn frames() -> (String, Values) {
    let mut ordered = WindowStatement::default();
    ordered.order_by_expr(at!("id"), Order::Asc);
    let day = SimpleExpr::from("1 day").cast_as_type(TypeName::new(n("interval")));
    let clauses: [FrameClause; 8] = [
        FrameType::Rows
            .preceding(1i64)
            .and_following(1i64)
            .exclude(FrameExclusion::CurrentRow),
        FrameType::Rows.unbounded_preceding().into(),
        FrameType::Groups.current_row().and_unbounded_following(),
        FrameType::Range.preceding(day).and_current_row(),
        FrameType::Rows.following(1i64).and_following(3i64),
        FrameType::Rows.preceding(3i64).and_preceding(1i64),
        FrameType::Rows.current_row().exclude(FrameExclusion::Ties),
        FrameType::Groups
            .unbounded_preceding()
            .and_unbounded_following()
            .exclude(FrameExclusion::NoOthers),
    ];
    let mut select = Query::select();
    for clause in clauses {
        let mut window = ordered.clone();
        window.frame(clause);
        select.expr_window(Func::sum(col!("w")), window);
    }
    select.from(reading!()).build()
}

fn functions() -> (String, Values) {
    let mut ordered = WindowStatement::default();
    ordered.order_by_expr(col!("id"), Order::Asc);
    let none = Vec::<SimpleExpr>::new;
    let mut select = Query::select();
    for call in [
        named!("rank", none()),
        named!("dense_rank", none()),
        named!("percent_rank", none()),
        named!("cume_dist", none()),
        named!("ntile", [SimpleExpr::from(4i32)]),
        named!("first_value", [col!("w")]),
        named!("last_value", [col!("w")]),
        named!("nth_value", [col!("w"), SimpleExpr::from(2i32)]),
        named!("lag", [col!("w")]),
    ] {
        select.expr_window(call, ordered.clone());
    }
    select
        .expr_window_as(
            named!(
                "lead",
                [col!("w"), SimpleExpr::from(1i32), SimpleExpr::from(0i64)]
            ),
            ordered,
            n("next"),
        )
        .from(table!("t"))
        .build()
}

fn json_aggregates() -> (String, Values) {
    let mut by_key = WindowStatement::default();
    by_key.add_partition_by(col!("k"));
    let mut ordered = WindowStatement::default();
    ordered.order_by_expr(col!("k"), Order::Asc);
    Query::select()
        .expr_window(
            Func::json_arrayagg(col!("v")).order_by(col!("v"), Order::Asc),
            by_key,
        )
        .expr_window_name_as(
            Func::json_objectagg(col!("k"), col!("v")),
            n("w"),
            n("pairs"),
        )
        .from(table!("t"))
        .window(n("w"), ordered)
        .build()
}

fn ranges() -> (String, Values) {
    let noon: jiff::Timestamp = "2026-10-09T12:00:00Z".parse().expect("an instant");
    let midnight: jiff::Timestamp = "2026-10-09T00:00:00Z".parse().expect("an instant");
    let during = Value::Range(
        RangeType::TimestampTz,
        Some(Box::new(Range::new(
            Bound::Included(midnight.into()),
            Bound::Unbounded,
        ))),
    );
    let seats = Value::Range(
        RangeType::Int4,
        Some(Box::new(Range::new(
            Bound::Included(1i32.into()),
            Bound::Included(10i32.into()),
        ))),
    );
    let spans = Value::Multirange(
        RangeType::Int4,
        Some(Box::new(Multirange::from(vec![
            Range::new(Bound::Included(1i32.into()), Bound::Excluded(3i32.into())),
            Range::new(Bound::Included(5i32.into()), Bound::Excluded(8i32.into())),
        ]))),
    );
    let tags = Value::Array(
        ArrayType::String,
        Some(Box::new(vec!["a".into(), "b".into()])),
    );
    let weights = Expr::expr(SimpleExpr::from("[0.5,1.5)"))
        .as_range(TypeName::new(n("floatrange")).schema(n("app")));
    let condition = col!("during")
        .binary(
            BinOper::Contains,
            SimpleExpr::from(noon).cast_as_type(TypeName::new(n("timestamptz"))),
        )
        .and(col!("during").binary(BinOper::Overlap, during))
        .and(col!("seats").binary(BinOper::Contained, seats))
        .and(col!("spans").binary(BinOper::Overlap, spans))
        .and(col!("tags").binary(BinOper::Contains, tags))
        .and(col!("weights").binary(BinOper::Overlap, weights));
    SelectStatement::new()
        .column(pgorm::pgorm_query::Asterisk)
        .from(table!("booking"))
        .cond_where(condition)
        .build()
}

fn range_positions() -> (String, Values) {
    let int4 = |lower: i32, upper: i32| {
        Range::new(Bound::Included(lower.into()), Bound::Excluded(upper.into()))
    };
    let seats = |lower, upper| Value::Range(RangeType::Int4, Some(Box::new(int4(lower, upper))));
    let spans = |lower, upper| {
        Value::Multirange(
            RangeType::Int4,
            Some(Box::new(Multirange::from(vec![int4(lower, upper)]))),
        )
    };
    let condition = col!("seats")
        .binary(BinOper::LShift, seats(5, 8))
        .and(col!("seats").binary(BinOper::RShift, seats(0, 1)))
        .and(col!("seats").binary(BinOper::NotExtendRightOf, seats(1, 6)))
        .and(col!("spans").binary(BinOper::NotExtendLeftOf, spans(0, 2)))
        .and(col!("spans").binary(BinOper::AdjacentTo, spans(6, 7)));
    SelectStatement::new()
        .column(pgorm::pgorm_query::Asterisk)
        .from(table!("booking"))
        .cond_where(condition)
        .build()
}

/// A WINDOW clause of three entries, each extended window built on the one
/// before it.
fn clause_entries() -> (String, Values) {
    let mut by_kind = WindowStatement::default();
    by_kind.add_partition_by(at!("kind"));
    Query::select()
        .expr_window_name(Func::sum(at!("weight")), n("running"))
        .expr_window_name_as(
            named!("rank", Vec::<SimpleExpr>::new()),
            n("ordered"),
            n("rank"),
        )
        .from(reading!())
        .window(n("by_kind"), by_kind)
        .window(
            n("ordered"),
            ExtendedWindow::new(n("by_kind"))
                .order_by_expr(at!("at"), Order::Asc)
                .to_owned(),
        )
        .window(
            n("running"),
            ExtendedWindow::new(n("ordered"))
                .frame(FrameType::Rows.preceding(1i64).and_current_row())
                .to_owned(),
        )
        .build()
}

/// Inline windows built on an entry of the WINDOW clause, one ordering and
/// framing it, one copying it bare under an alias.
fn inline_extended() -> (String, Values) {
    let mut by_kind = WindowStatement::default();
    by_kind.add_partition_by(at!("kind"));
    Query::select()
        .expr_window(
            Func::sum(at!("weight")),
            ExtendedWindow::new(n("by_kind"))
                .order_by_expr(at!("at"), Order::Asc)
                .frame(FrameType::Rows.preceding(1i64).and_current_row())
                .to_owned(),
        )
        .expr_window_as(
            named!("rank", Vec::<SimpleExpr>::new()),
            ExtendedWindow::new(n("by_kind")),
            n("rank"),
        )
        .from(reading!())
        .window(n("by_kind"), by_kind)
        .build()
}

pub(super) fn cases() -> Vec<(&'static str, (String, Values))> {
    vec![
        ("window-inline-and-named", inline_and_named()),
        ("window-clause-entries", clause_entries()),
        ("window-inline-extended", inline_extended()),
        ("window-frames", frames()),
        ("window-functions", functions()),
        ("window-json-aggregates", json_aggregates()),
        ("ranges", ranges()),
        ("range-positions", range_positions()),
    ]
}
