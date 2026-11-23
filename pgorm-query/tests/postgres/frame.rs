use super::*;
use crate::oracle::{assert_eq, parsed_nodes};

// PostgreSQL's `frameOptions` bits (parsenodes.h), which is how the parser
// records a frame: the mode, whether BETWEEN was written, what each bound is,
// and the exclusion.
const RANGE: i64 = 0x00002;
const ROWS: i64 = 0x00004;
const GROUPS: i64 = 0x00008;
const BETWEEN: i64 = 0x00010;
const START_UNBOUNDED_PRECEDING: i64 = 0x00020;
const END_UNBOUNDED_FOLLOWING: i64 = 0x00100;
const START_CURRENT_ROW: i64 = 0x00200;
const END_CURRENT_ROW: i64 = 0x00400;
const START_OFFSET_PRECEDING: i64 = 0x00800;
const END_OFFSET_PRECEDING: i64 = 0x01000;
const START_OFFSET_FOLLOWING: i64 = 0x02000;
const END_OFFSET_FOLLOWING: i64 = 0x04000;
const EXCLUDE_CURRENT_ROW: i64 = 0x08000;
const EXCLUDE_GROUP: i64 = 0x10000;
const EXCLUDE_TIES: i64 = 0x20000;
/// Set on every explicitly written frame.
const NONDEFAULT: i64 = 0x00001;

/// `SELECT SUM("size_w") OVER ( ORDER BY "font_size" ASC <frame> ) FROM "character"`.
fn framed(frame: impl Into<FrameClause>) -> SelectStatement {
    Query::select()
        .from(Char::Table)
        .expr_window(
            Func::sum(Expr::col(Char::SizeW)),
            WindowStatement::new()
                .order_by(Char::FontSize, Order::Asc)
                .frame(frame)
                .take(),
        )
        .take()
}

/// The frame text between the window's `ORDER BY` and its closing parenthesis.
fn frame_text(sql: &str) -> &str {
    let order = r#""font_size" ASC "#;
    let start = sql.find(order).expect("an ordered window") + order.len();
    let end = sql.rfind(" )").expect("a closed window");
    &sql[start..end]
}

/// The one window in `sql`, as the parser read it.
fn window_def(sql: &str) -> serde_json::Value {
    let mut calls = parsed_nodes(sql, "FuncCall");
    assert_eq!(calls.len(), 1, "exactly one call in {sql}");
    calls.remove(0)["over"].clone()
}

fn frame_options(sql: &str) -> i64 {
    window_def(sql)["frame_options"].as_i64().expect("a frame")
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    a preceding or current-row start stands alone,
// and each start offers the ends that may follow it
// [spec:pgorm:req:sql.render.window+8/test]    the mode, then the start alone or `BETWEEN start
// AND end`, each held to the parse tree's `frameOptions`
#[test]
fn each_admitted_pairing_renders_its_frame() {
    let cases: [(FrameClause, &str, i64); 11] = [
        (
            FrameType::Rows.unbounded_preceding().into(),
            "ROWS UNBOUNDED PRECEDING",
            ROWS | START_UNBOUNDED_PRECEDING | END_CURRENT_ROW,
        ),
        (
            FrameType::Rows.preceding(2).into(),
            "ROWS 2 PRECEDING",
            ROWS | START_OFFSET_PRECEDING | END_CURRENT_ROW,
        ),
        (
            FrameType::Groups.current_row().into(),
            "GROUPS CURRENT ROW",
            GROUPS | START_CURRENT_ROW | END_CURRENT_ROW,
        ),
        (
            FrameType::Range.unbounded_preceding().and_preceding(1),
            "RANGE BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING",
            RANGE | BETWEEN | START_UNBOUNDED_PRECEDING | END_OFFSET_PRECEDING,
        ),
        (
            FrameType::Rows.preceding(3).and_preceding(1),
            "ROWS BETWEEN 3 PRECEDING AND 1 PRECEDING",
            ROWS | BETWEEN | START_OFFSET_PRECEDING | END_OFFSET_PRECEDING,
        ),
        (
            FrameType::Rows.preceding(1).and_current_row(),
            "ROWS BETWEEN 1 PRECEDING AND CURRENT ROW",
            ROWS | BETWEEN | START_OFFSET_PRECEDING | END_CURRENT_ROW,
        ),
        (
            FrameType::Groups.preceding(1).and_following(1),
            "GROUPS BETWEEN 1 PRECEDING AND 1 FOLLOWING",
            GROUPS | BETWEEN | START_OFFSET_PRECEDING | END_OFFSET_FOLLOWING,
        ),
        (
            FrameType::Range
                .unbounded_preceding()
                .and_unbounded_following(),
            "RANGE BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING",
            RANGE | BETWEEN | START_UNBOUNDED_PRECEDING | END_UNBOUNDED_FOLLOWING,
        ),
        (
            FrameType::Rows.current_row().and_current_row(),
            "ROWS BETWEEN CURRENT ROW AND CURRENT ROW",
            ROWS | BETWEEN | START_CURRENT_ROW | END_CURRENT_ROW,
        ),
        (
            FrameType::Groups.current_row().and_following(2),
            "GROUPS BETWEEN CURRENT ROW AND 2 FOLLOWING",
            GROUPS | BETWEEN | START_CURRENT_ROW | END_OFFSET_FOLLOWING,
        ),
        (
            FrameType::Rows.following(1).and_unbounded_following(),
            "ROWS BETWEEN 1 FOLLOWING AND UNBOUNDED FOLLOWING",
            ROWS | BETWEEN | START_OFFSET_FOLLOWING | END_UNBOUNDED_FOLLOWING,
        ),
    ];

    for (frame, text, options) in cases {
        let sql = framed(frame).to_string();
        assert_eq!(frame_text(&sql), text);
        assert_eq!(frame_options(&sql), options | NONDEFAULT, "{sql}");
    }
    assert_eq!(
        frame_options(&framed(FrameType::Rows.following(1).and_following(3)).to_string()),
        NONDEFAULT | ROWS | BETWEEN | START_OFFSET_FOLLOWING | END_OFFSET_FOLLOWING
    );
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    each exclusion is a method of the finished
// frame, a lone start included
// [spec:pgorm:req:sql.render.window+8/test]    ` EXCLUDE …` after the bounds
#[test]
fn each_exclusion_renders_after_the_bounds() {
    let whole = || {
        FrameType::Rows
            .unbounded_preceding()
            .and_unbounded_following()
    };
    let bounds = ROWS | BETWEEN | START_UNBOUNDED_PRECEDING | END_UNBOUNDED_FOLLOWING;

    for (exclusion, text, bit) in [
        (
            FrameExclusion::CurrentRow,
            "EXCLUDE CURRENT ROW",
            EXCLUDE_CURRENT_ROW,
        ),
        (FrameExclusion::Group, "EXCLUDE GROUP", EXCLUDE_GROUP),
        (FrameExclusion::Ties, "EXCLUDE TIES", EXCLUDE_TIES),
        (FrameExclusion::NoOthers, "EXCLUDE NO OTHERS", 0),
    ] {
        let sql = framed(whole().exclude(exclusion)).to_string();
        assert_eq!(
            frame_text(&sql),
            format!("ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING {text}")
        );
        assert_eq!(frame_options(&sql), NONDEFAULT | bounds | bit, "{sql}");
    }

    // A lone start takes the clause too, and a second call replaces the first.
    let sql = framed(
        FrameType::Groups
            .preceding(1)
            .exclude(FrameExclusion::Group)
            .exclude(FrameExclusion::Ties),
    )
    .to_string();
    assert_eq!(frame_text(&sql), "GROUPS 1 PRECEDING EXCLUDE TIES");
    assert_eq!(
        frame_options(&sql),
        NONDEFAULT | GROUPS | START_OFFSET_PRECEDING | END_CURRENT_ROW | EXCLUDE_TIES
    );
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    an offset is any expression: a bound value,
// a cast, arithmetic
// [spec:pgorm:req:sql.render.window+8/test]    the offset renders through the expression path,
// `$N` when bound
#[test]
fn an_offset_is_an_expression() {
    let interval = Expr::val("1 day").cast_as(Name::runtime("interval"));
    let sql = framed(
        FrameType::Range
            .preceding(interval.clone())
            .and_current_row(),
    )
    .to_string();
    assert_eq!(
        frame_text(&sql),
        "RANGE BETWEEN CAST('1 day' AS interval) PRECEDING AND CURRENT ROW"
    );
    assert_eq!(
        window_def(&sql)["start_offset"]["TypeCast"]["type_name"]["names"][1]["String"]["sval"],
        "interval",
        "the keyword type reads as pg_catalog.interval"
    );

    let (sql, values) = framed(FrameType::Range.preceding(interval).and_following(2)).build();
    assert_eq!(
        frame_text(&sql),
        "RANGE BETWEEN CAST($1::text AS interval) PRECEDING AND $2 FOLLOWING"
    );
    assert_eq!(
        values,
        Values(vec![
            Value::String(Some(Box::new("1 day".to_owned()))),
            Value::Int(Some(2))
        ])
    );
    assert!(
        window_def(&sql)["end_offset"].get("ParamRef").is_some(),
        "{sql}"
    );

    // Arithmetic needs no parentheses of its own: `PRECEDING` ends the offset.
    let sql = framed(
        FrameType::Groups
            .preceding(Expr::val(1).add(1))
            .and_current_row(),
    )
    .to_string();
    assert_eq!(
        frame_text(&sql),
        "GROUPS BETWEEN 1 + 1 PRECEDING AND CURRENT ROW"
    );
    assert_eq!(
        window_def(&sql)["start_offset"]["AExpr"]["name"][0]["String"]["sval"],
        "+"
    );
}
