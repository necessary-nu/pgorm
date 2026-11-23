use super::*;
use crate::oracle::{assert_eq, parsed_nodes};
use pgorm_query::error::Error;

fn counted() -> FunctionCall {
    Func::count(Expr::col(Char::Id))
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    PARTITION BY accumulates from all four entry
// points
// [spec:pgorm:req:sql.render.window+8/test]    an inline window renders ` OVER ( … )`
#[test]
fn window_1() {
    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window(counted(), WindowStatement::partition_by(Char::FontSize))
            .to_string(),
        r#"SELECT COUNT("id") OVER ( PARTITION BY "font_size" ) FROM "character""#
    );

    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window_as(
                counted(),
                WindowStatement::partition_by(Char::FontSize)
                    .partition_by(Char::SizeW)
                    .partition_by_columns([Char::SizeH])
                    .take(),
                Name::runtime("C")
            )
            .to_string(),
        [
            r#"SELECT COUNT("id") OVER ("#,
            r#"PARTITION BY "font_size", "size_w", "size_h" ) AS "C""#,
            r#"FROM "character""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    ORDER BY comes from the shared
// `OrderedStatement` trait
// [spec:pgorm:req:sql.render.window+8/test]    ` PARTITION BY … ORDER BY …`
#[test]
fn window_2() {
    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window_as(
                counted(),
                WindowStatement::partition_by(Char::FontSize)
                    .order_by(Char::Id, Order::Asc)
                    .order_by(Char::SizeW, Order::Desc)
                    .take(),
                Name::runtime("C")
            )
            .to_string(),
        [
            r#"SELECT COUNT("id") OVER ("#,
            r#"PARTITION BY "font_size" ORDER BY "id" ASC, "size_w" DESC ) AS "C""#,
            r#"FROM "character""#,
        ]
        .join(" ")
    );

    // A window with no partition at all is just its ORDER BY.
    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window(
                counted(),
                WindowStatement::new().order_by(Char::Id, Order::Asc).take()
            )
            .to_string(),
        r#"SELECT COUNT("id") OVER (  ORDER BY "id" ASC ) FROM "character""#
    );

    // An empty specification still renders its parentheses.
    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window(counted(), WindowStatement::new())
            .to_string(),
        r#"SELECT COUNT("id") OVER (  ) FROM "character""#
    );
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    a lone start is a whole frame, and
// `and_*` gives one an end, for either frame type
// [spec:pgorm:req:sql.render.window+8/test]    ` RANGE `/` ROWS ` then `BETWEEN start AND end` or
// the start bound alone
#[test]
fn window_3() {
    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window(
                counted(),
                WindowStatement::partition_by(Char::FontSize)
                    .frame(FrameType::Rows.unbounded_preceding())
                    .take()
            )
            .to_string(),
        [
            r#"SELECT COUNT("id") OVER ("#,
            r#"PARTITION BY "font_size" ROWS UNBOUNDED PRECEDING )"#,
            r#"FROM "character""#,
        ]
        .join(" ")
    );

    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window(
                counted(),
                WindowStatement::partition_by(Char::FontSize)
                    .frame(
                        FrameType::Range
                            .unbounded_preceding()
                            .and_unbounded_following()
                    )
                    .take()
            )
            .to_string(),
        [
            r#"SELECT COUNT("id") OVER ("#,
            r#"PARTITION BY "font_size" RANGE BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING )"#,
            r#"FROM "character""#,
        ]
        .join(" ")
    );

    // The last `frame` call wins.
    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window(
                counted(),
                WindowStatement::partition_by(Char::FontSize)
                    .frame(
                        FrameType::Range
                            .unbounded_preceding()
                            .and_unbounded_following()
                    )
                    .frame(FrameType::Rows.current_row())
                    .take()
            )
            .to_string(),
        [
            r#"SELECT COUNT("id") OVER ("#,
            r#"PARTITION BY "font_size" ROWS CURRENT ROW )"#,
            r#"FROM "character""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.render.window+8/test]    a bounded offset renders the value, a space, then
// the keyword
#[test]
fn window_4() {
    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window(
                counted(),
                WindowStatement::partition_by(Char::FontSize)
                    .frame(FrameType::Rows.preceding(2).and_following(3))
                    .take()
            )
            .build(),
        (
            [
                r#"SELECT COUNT("id") OVER ("#,
                r#"PARTITION BY "font_size" ROWS BETWEEN $1 PRECEDING AND $2 FOLLOWING )"#,
                r#"FROM "character""#,
            ]
            .join(" "),
            Values(vec![Value::Int(Some(2)), Value::Int(Some(3))])
        )
    );

    // Inline, the same shape drops the placeholder for the literal.
    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window(
                counted(),
                WindowStatement::partition_by(Char::FontSize)
                    .frame(FrameType::Rows.preceding(2))
                    .take()
            )
            .to_string(),
        [
            r#"SELECT COUNT("id") OVER ("#,
            r#"PARTITION BY "font_size" ROWS 2 PRECEDING )"#,
            r#"FROM "character""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    all three frame modes render, GROUPS
// included, and an offset means the same to the renderer in each
#[test]
fn every_frame_mode_renders_its_keyword() {
    let framed = |r#type: FrameType| {
        Query::select()
            .from(Char::Table)
            .expr_window(
                counted(),
                WindowStatement::partition_by(Char::FontSize)
                    .order_by(Char::SizeW, Order::Asc)
                    .frame(r#type.preceding(1).and_following(1))
                    .take(),
            )
            .to_string()
    };

    for (r#type, keyword) in [
        (FrameType::Range, "RANGE"),
        (FrameType::Rows, "ROWS"),
        (FrameType::Groups, "GROUPS"),
    ] {
        assert_eq!(
            framed(r#type),
            [
                r#"SELECT COUNT("id") OVER ( PARTITION BY "font_size" ORDER BY "size_w" ASC"#,
                &format!("{keyword} BETWEEN 1 PRECEDING AND 1 FOLLOWING )"),
                r#"FROM "character""#,
            ]
            .join(" ")
        );
    }
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    `WindowSelectType::Name` references a window
// declared at statement level with `SelectStatement::window`
// [spec:pgorm:req:sql.render.window+8/test]    a named reference renders ` OVER "name"` and its
// declaration ` WINDOW "name" AS ( … )`
#[test]
fn window_5() {
    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window_name(counted(), Name::runtime("w"))
            .window(
                Name::runtime("w"),
                WindowStatement::partition_by(Char::FontSize)
            )
            .to_string(),
        [
            r#"SELECT COUNT("id") OVER "w" FROM "character""#,
            r#"WINDOW "w" AS ( PARTITION BY "font_size" )"#,
        ]
        .join(" ")
    );

    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window_name_as(counted(), Name::runtime("w"), Name::runtime("C"))
            .window(
                Name::runtime("w"),
                WindowStatement::partition_by(Char::FontSize)
            )
            .to_string(),
        [
            r#"SELECT COUNT("id") OVER "w" AS "C" FROM "character""#,
            r#"WINDOW "w" AS ( PARTITION BY "font_size" )"#,
        ]
        .join(" ")
    );
}

/// Each entry of the WINDOW clause as the parser read it: its name, and the
/// base it extends ("" for a window of its own).
fn window_clause(sql: &str) -> Vec<(String, String)> {
    let select = parsed_nodes(sql, "SelectStmt").remove(0);
    select["window_clause"]
        .as_array()
        .expect("a WINDOW clause")
        .iter()
        .map(|entry| {
            let def = &entry["WindowDef"];
            let text = |field: &str| def[field].as_str().unwrap_or_default().to_owned();
            (text("name"), text("refname"))
        })
        .collect()
}

fn named(name: &str) -> Name {
    Name::runtime(name)
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    the WINDOW clause holds every named window,
// each call appending one
// [spec:pgorm:req:sql.render.window+8/test]    ` WINDOW "a" AS ( … ), "b" AS ( … )` in call order
#[test]
fn every_named_window_renders_in_call_order() {
    let query = Query::select()
        .from(Char::Table)
        .expr_window_name(counted(), named("w2"))
        .expr_window_name(Func::max(Expr::col(Char::Id)), named("w1"))
        .window(named("w1"), WindowStatement::partition_by(Char::FontSize))
        .window(named("w2"), WindowStatement::partition_by(Char::SizeW))
        .window(named("unused"), WindowStatement::new())
        .take();
    let sql = query.to_string();
    assert_eq!(
        sql,
        [
            r#"SELECT COUNT("id") OVER "w2", MAX("id") OVER "w1" FROM "character""#,
            r#"WINDOW "w1" AS ( PARTITION BY "font_size" ),"#,
            r#""w2" AS ( PARTITION BY "size_w" ), "unused" AS (  )"#,
        ]
        .join(" ")
    );
    std::assert_eq!(
        window_clause(&sql),
        [
            ("w1".to_owned(), String::new()),
            ("w2".to_owned(), String::new()),
            ("unused".to_owned(), String::new()),
        ]
    );
    std::assert_eq!(query.check_windows(), Ok(()));
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    an extended window names an earlier entry and
// adds an ORDER BY and a frame
// [spec:pgorm:req:sql.render.window+8/test]    `( "base" ORDER BY … <frame> )`, the base where a
// window of its own writes its PARTITION BY
#[test]
fn extended_window_renders_its_base_first() {
    let query = Query::select()
        .from(Char::Table)
        .expr_window_name(counted(), named("framed"))
        .window(
            named("parts"),
            WindowStatement::partition_by(Char::FontSize),
        )
        .window(
            named("ordered"),
            ExtendedWindow::new(named("parts"))
                .order_by(Char::SizeW, Order::Desc)
                .to_owned(),
        )
        .window(
            named("framed"),
            ExtendedWindow::new(named("ordered"))
                .frame(FrameType::Rows.preceding(1).and_following(2))
                .to_owned(),
        )
        .window(named("copy"), ExtendedWindow::new(named("parts")))
        .take();
    let sql = query.to_string();
    assert_eq!(
        sql,
        [
            r#"SELECT COUNT("id") OVER "framed" FROM "character""#,
            r#"WINDOW "parts" AS ( PARTITION BY "font_size" ),"#,
            r#""ordered" AS ( "parts" ORDER BY "size_w" DESC ),"#,
            r#""framed" AS ( "ordered" ROWS BETWEEN 1 PRECEDING AND 2 FOLLOWING ),"#,
            r#""copy" AS ( "parts" )"#,
        ]
        .join(" ")
    );
    std::assert_eq!(
        window_clause(&sql),
        [
            ("parts".to_owned(), String::new()),
            ("ordered".to_owned(), "parts".to_owned()),
            ("framed".to_owned(), "ordered".to_owned()),
            ("copy".to_owned(), "parts".to_owned()),
        ]
    );
    std::assert_eq!(query.check_windows(), Ok(()));

    // Bound, a frame offset in the clause takes its place among the
    // statement's parameters.
    let (built, values) = query.build();
    assert!(
        built.contains(r#""framed" AS ( "ordered" ROWS BETWEEN $1 PRECEDING AND $2 FOLLOWING )"#),
        "{built}"
    );
    std::assert_eq!(
        values,
        Values(vec![Value::Int(Some(1)), Value::Int(Some(2))])
    );
}

/// A statement reading `over` through `OVER "w"`, with `windows` declared.
fn declaring(windows: Vec<(&str, WindowDefinition)>) -> SelectStatement {
    let mut query = Query::select();
    query
        .from(Char::Table)
        .expr_window_name(counted(), named("w"));
    for (name, window) in windows {
        query.window(named(name), window);
    }
    query
}

fn by_size() -> WindowDefinition {
    WindowStatement::partition_by(Char::FontSize).into()
}

fn ordered_by_id() -> WindowDefinition {
    WindowStatement::new()
        .order_by(Char::Id, Order::Asc)
        .take()
        .into()
}

fn extending(base: &str) -> ExtendedWindow {
    ExtendedWindow::new(named(base))
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    `check_windows` refuses each clause PostgreSQL
// refuses, every one of which parses
#[test]
fn check_windows_refuses_what_postgres_refuses() {
    let refused = |windows: Vec<(&str, WindowDefinition)>| {
        let query = declaring(windows);
        crate::oracle::assert_parses(&query.to_string());
        query.check_windows().expect_err("a refused clause")
    };
    let pair = |window: &str, base: &str| (Some(window.to_owned()), base.to_owned());

    std::assert_eq!(
        refused(vec![("w", by_size()), ("w", ordered_by_id())]),
        Error::WindowRedefined {
            name: "w".to_owned()
        }
    );
    std::assert_eq!(
        refused(vec![("v", by_size())]),
        Error::WindowUndefined {
            name: "w".to_owned()
        }
    );

    let base_error = |error: Error| match error {
        Error::WindowBaseUndefined { window, base } => ("undefined", (window, base)),
        Error::WindowFrameCopied { window, base } => ("frame", (window, base)),
        Error::WindowOrderOverridden { window, base } => ("order", (window, base)),
        other => panic!("not a base refusal: {other:?}"),
    };
    // A base named later in the clause is no earlier entry.
    std::assert_eq!(
        base_error(refused(vec![
            ("w", extending("v").into()),
            ("v", by_size())
        ])),
        ("undefined", pair("w", "v"))
    );
    std::assert_eq!(
        base_error(refused(vec![
            (
                "v",
                WindowStatement::new()
                    .frame(FrameType::Rows.current_row())
                    .take()
                    .into()
            ),
            ("w", extending("v").into()),
        ])),
        ("frame", pair("w", "v"))
    );
    std::assert_eq!(
        base_error(refused(vec![
            ("v", ordered_by_id()),
            (
                "w",
                extending("v")
                    .order_by(Char::SizeW, Order::Asc)
                    .to_owned()
                    .into()
            ),
        ])),
        ("order", pair("w", "v"))
    );
    // An ORDER BY copied down a chain still orders the copy, and a frame
    // set on an extended window still bars extending it.
    std::assert_eq!(
        base_error(refused(vec![
            ("u", ordered_by_id()),
            ("v", extending("u").into()),
            (
                "w",
                extending("v")
                    .order_by(Char::SizeW, Order::Asc)
                    .to_owned()
                    .into()
            ),
        ])),
        ("order", pair("w", "v"))
    );
    std::assert_eq!(
        base_error(refused(vec![
            ("u", by_size()),
            (
                "v",
                extending("u")
                    .frame(FrameType::Rows.current_row())
                    .to_owned()
                    .into()
            ),
            ("w", extending("v").into()),
        ])),
        ("frame", pair("w", "v"))
    );

    // What PostgreSQL accepts passes: an ORDER BY on an unordered base, a
    // frame on an ordered one, and a copy of an ordered base.
    for windows in [
        vec![
            ("u", by_size()),
            (
                "w",
                extending("u")
                    .order_by(Char::Id, Order::Asc)
                    .to_owned()
                    .into(),
            ),
        ],
        vec![
            ("u", ordered_by_id()),
            (
                "w",
                extending("u")
                    .frame(FrameType::Rows.current_row())
                    .to_owned()
                    .into(),
            ),
        ],
        vec![
            ("u", ordered_by_id()),
            ("v", extending("u").into()),
            ("w", extending("v").into()),
        ],
    ] {
        std::assert_eq!(declaring(windows).check_windows(), Ok(()));
    }
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    window names are compared as the text they
// render, case-exactly, whichever types minted them
#[test]
fn window_names_compare_by_their_text() {
    let w = alias("w");
    let mut query = Query::select();
    query
        .from(Char::Table)
        .expr_window_name(counted(), w)
        .window(named("w"), by_size());
    std::assert_eq!(query.check_windows(), Ok(()));

    query.window(w, ordered_by_id());
    std::assert_eq!(
        query.check_windows(),
        Err(Error::WindowRedefined {
            name: "w".to_owned()
        })
    );

    let cased = declaring(vec![("W", by_size())]);
    std::assert_eq!(
        cased.check_windows(),
        Err(Error::WindowUndefined {
            name: "w".to_owned()
        })
    );
}

/// The window each windowed call reads, as the parser read it: the base an
/// inline window names ("" for none) and how many ORDER BY items it adds.
fn inline_windows(sql: &str) -> Vec<(String, usize)> {
    parsed_nodes(sql, "FuncCall")
        .iter()
        .filter_map(|call| call.get("over"))
        .map(|over| {
            let refname = over["refname"].as_str().unwrap_or_default().to_owned();
            let orders = over["order_clause"].as_array().map_or(0, Vec::len);
            (refname, orders)
        })
        .collect()
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    an inline window can build on an entry of the
// WINDOW clause, through either inline constructor, with or without an alias
// [spec:pgorm:req:sql.render.window+8/test]    ` OVER ( "base" ORDER BY … <frame> )`, written as
// an extended entry is
#[test]
fn inline_window_builds_on_a_named_one() {
    let query = Query::select()
        .from(Char::Table)
        .expr_window(
            counted(),
            extending("parts")
                .order_by(Char::Id, Order::Asc)
                .frame(FrameType::Rows.unbounded_preceding())
                .to_owned(),
        )
        .expr_window_as(
            Func::max(Expr::col(Char::Id)),
            extending("ordered"),
            named("top"),
        )
        .window(
            named("parts"),
            WindowStatement::partition_by(Char::FontSize),
        )
        .window(
            named("ordered"),
            WindowStatement::new()
                .order_by(Char::SizeW, Order::Desc)
                .take(),
        )
        .take();
    let sql = query.to_string();
    assert_eq!(
        sql,
        [
            r#"SELECT COUNT("id") OVER ( "parts" ORDER BY "id" ASC ROWS UNBOUNDED PRECEDING ),"#,
            r#"MAX("id") OVER ( "ordered" ) AS "top" FROM "character""#,
            r#"WINDOW "parts" AS ( PARTITION BY "font_size" ),"#,
            r#""ordered" AS (  ORDER BY "size_w" DESC )"#,
        ]
        .join(" ")
    );
    std::assert_eq!(
        inline_windows(&sql),
        [("parts".to_owned(), 1), ("ordered".to_owned(), 0)]
    );
    std::assert_eq!(query.check_windows(), Ok(()));
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    `check_windows` holds an inline window to
// what an extended entry is held to, its base any entry of the clause
#[test]
fn check_windows_refuses_inline_bases_postgres_refuses() {
    let over = |window: ExtendedWindow, windows: Vec<(&str, WindowDefinition)>| {
        let mut query = Query::select();
        query.from(Char::Table).expr_window(counted(), window);
        for (name, definition) in windows {
            query.window(named(name), definition);
        }
        crate::oracle::assert_parses(&query.to_string());
        query.check_windows()
    };
    let refused = |window, windows| over(window, windows).expect_err("a refused window");
    let ordered = || extending("v").order_by(Char::SizeW, Order::Asc).to_owned();

    std::assert_eq!(
        refused(extending("v"), vec![("u", by_size())]),
        Error::WindowBaseUndefined {
            window: None,
            base: "v".to_owned()
        }
    );
    std::assert_eq!(
        refused(
            extending("v"),
            vec![(
                "v",
                WindowStatement::new()
                    .frame(FrameType::Rows.current_row())
                    .take()
                    .into()
            )]
        ),
        Error::WindowFrameCopied {
            window: None,
            base: "v".to_owned()
        }
    );
    std::assert_eq!(
        refused(ordered(), vec![("v", ordered_by_id())]),
        Error::WindowOrderOverridden {
            window: None,
            base: "v".to_owned()
        }
    );
    // An ORDER BY copied down to the base orders it as its own does.
    std::assert_eq!(
        refused(
            ordered(),
            vec![("u", ordered_by_id()), ("v", extending("u").into())]
        ),
        Error::WindowOrderOverridden {
            window: None,
            base: "v".to_owned()
        }
    );
    // The inline window's base may be any entry: the clause precedes it, so
    // none is later than it.
    std::assert_eq!(
        over(ordered(), vec![("v", by_size()), ("w", by_size())]),
        Ok(())
    );
    std::assert_eq!(
        over(
            extending("v")
                .frame(FrameType::Rows.current_row())
                .to_owned(),
            vec![("v", ordered_by_id())]
        ),
        Ok(())
    );
    // An entry's refusal still comes first, naming the entry.
    std::assert_eq!(
        refused(
            extending("v"),
            vec![("v", by_size()), ("w", extending("x").into())]
        ),
        Error::WindowBaseUndefined {
            window: Some("w".to_owned()),
            base: "x".to_owned()
        }
    );
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    each refusal says what PostgreSQL would, an
// inline window's naming only its base
#[test]
fn window_refusals_name_the_windows() {
    let (w, v) = (|| "w".to_owned(), || "v".to_owned());
    let messages = [
        Error::WindowRedefined { name: w() },
        Error::WindowUndefined { name: w() },
        Error::WindowBaseUndefined {
            window: Some(w()),
            base: v(),
        },
        Error::WindowFrameCopied {
            window: Some(w()),
            base: v(),
        },
        Error::WindowOrderOverridden {
            window: Some(w()),
            base: v(),
        },
        Error::WindowBaseUndefined {
            window: None,
            base: v(),
        },
        Error::WindowFrameCopied {
            window: None,
            base: v(),
        },
        Error::WindowOrderOverridden {
            window: None,
            base: v(),
        },
    ]
    .map(|error| error.to_string());
    std::assert_eq!(
        messages,
        [
            r#"the WINDOW clause names two windows "w""#,
            r#"no window "w" is declared in the WINDOW clause"#,
            r#"window "w" extends "v", which no earlier entry of the WINDOW clause declares"#,
            r#"window "w" cannot extend "v", which has a frame"#,
            r#"window "w" cannot add an ORDER BY to "v", which already orders its rows"#,
            r#"the window after OVER extends "v", which the WINDOW clause does not declare"#,
            r#"the window after OVER cannot extend "v", which has a frame"#,
            r#"the window after OVER cannot add an ORDER BY to "v", which already orders its rows"#,
        ]
    );
}

// [spec:pgorm:def:sql.ast.select+6/test]    `take()` moves the WINDOW clause out with the rest
#[test]
fn take_moves_the_window_clause() {
    let mut query = declaring(vec![("w", by_size())]);
    let taken = query.take();
    assert!(taken.to_string().contains(r#"WINDOW "w""#));
    assert!(!query.to_string().contains("WINDOW"));
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    `take()` moves the contents out and leaves the
// builder empty
#[test]
fn window_7() {
    let mut window = WindowStatement::partition_by(Char::FontSize);
    window.order_by(Char::Id, Order::Asc);
    window.frame(FrameType::Rows.current_row());

    let taken = window.take();
    assert_eq!(window, WindowStatement::new());
    assert_ne!(taken, WindowStatement::new());
}

// [spec:pgorm:req:sql.render.select-order+7/test]    WINDOW sits with HAVING, ahead of the tail
// clauses that apply to the whole result
// [spec:pgorm:req:sql.render.window+8/test]
#[test]
fn window_clause_precedes_order_limit_lock() {
    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window_name(counted(), Name::runtime("w"))
            .add_group_by([Expr::col(Char::FontSize).into()])
            .and_having(Expr::col(Char::SizeW).gt(1))
            .window(
                Name::runtime("w"),
                WindowStatement::partition_by(Char::FontSize)
            )
            .order_by(Char::Id, Order::Asc)
            .limit(5)
            .offset(2)
            .to_string(),
        [
            r#"SELECT COUNT("id") OVER "w" FROM "character""#,
            r#"GROUP BY "font_size" HAVING "size_w" > 1"#,
            r#"WINDOW "w" AS ( PARTITION BY "font_size" )"#,
            r#"ORDER BY "id" ASC LIMIT 5 OFFSET 2"#,
        ]
        .join(" ")
    );

    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window_name(counted(), Name::runtime("w"))
            .window(
                Name::runtime("w"),
                WindowStatement::partition_by(Char::FontSize)
            )
            .lock(LockType::Update)
            .to_string(),
        [
            r#"SELECT COUNT("id") OVER "w" FROM "character""#,
            r#"WINDOW "w" AS ( PARTITION BY "font_size" ) FOR UPDATE"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.render.select-order+7/test]    a named window belongs to its own query level,
// so it is written before the set operation
#[test]
fn window_clause_precedes_union() {
    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window_name(counted(), Name::runtime("w"))
            .window(
                Name::runtime("w"),
                WindowStatement::partition_by(Char::FontSize)
            )
            .union(
                UnionType::All,
                Query::select().column(Char::Id).from(Char::Table).take()
            )
            .order_by(Char::Id, Order::Asc)
            .to_string(),
        [
            r#"SELECT COUNT("id") OVER "w" FROM "character""#,
            r#"WINDOW "w" AS ( PARTITION BY "font_size" )"#,
            r#"UNION ALL (SELECT "id" FROM "character")"#,
            r#"ORDER BY "id" ASC"#,
        ]
        .join(" ")
    );
}
