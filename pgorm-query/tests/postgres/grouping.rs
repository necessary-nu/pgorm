use super::*;
use crate::oracle::{assert_eq, parsed_nodes};

/// `SELECT "aspect" FROM "glyph" GROUP BY <element>`.
fn grouped(element: impl Into<GroupingElement>) -> String {
    Query::select()
        .column(Glyph::Aspect)
        .from(Glyph::Table)
        .group_by_element(element)
        .to_string()
}

fn aspect() -> Expr {
    Expr::col(Glyph::Aspect)
}

fn image() -> Expr {
    Expr::col(Glyph::Image)
}

/// libpg_query's `GroupingSetKind`, by the number it serialises as. The
/// grammar never builds the fifth kind, `GROUPING_SET_SIMPLE` (2): parse
/// analysis does, from a parenthesised row.
const EMPTY: i64 = 1;
const ROLLUP: i64 = 3;
const CUBE: i64 = 4;
const SETS: i64 = 5;

/// Each `GroupingSet` node in `sql`, outermost first, as `(kind, members)`.
fn grouping_sets(sql: &str) -> Vec<(i64, usize)> {
    parsed_nodes(sql, "GroupingSet")
        .iter()
        .map(|set| {
            (
                set["kind"].as_i64().expect("a kind"),
                set.get("content")
                    .and_then(serde_json::Value::as_array)
                    .map_or(0, Vec::len),
            )
        })
        .collect()
}

// [spec:pgorm:def:sql.ast.select.grouping/test]    `rollup` and `cube` take their expressions
// [spec:pgorm:req:sql.render.grouping+1/test]    `ROLLUP (…)` and `CUBE (…)`
#[test]
fn rollup_and_cube_render_their_keyword() {
    let sql = grouped(GroupingElement::rollup([aspect(), image()]));
    assert_eq!(
        sql,
        r#"SELECT "aspect" FROM "glyph" GROUP BY ROLLUP ("aspect", "image")"#
    );
    assert_eq!(grouping_sets(&sql), [(ROLLUP, 2)]);

    let sql = grouped(GroupingElement::cube([aspect(), image()]));
    assert_eq!(
        sql,
        r#"SELECT "aspect" FROM "glyph" GROUP BY CUBE ("aspect", "image")"#
    );
    assert_eq!(grouping_sets(&sql), [(CUBE, 2)]);
}

// [spec:pgorm:def:sql.ast.select.grouping/test]    `set` and `empty`, and `sets` taking its first
// element up front
// [spec:pgorm:req:sql.render.grouping+1/test]    `GROUPING SETS (…)` holding a bare expression, a
// parenthesised set, `()` and a nested element
#[test]
fn grouping_sets_hold_every_element_kind() {
    let sql = grouped(
        GroupingElement::sets(GroupingElement::set([aspect()]))
            .add(GroupingElement::set([aspect(), image()]))
            .add(GroupingElement::empty())
            .add(GroupingElement::cube([image()])),
    );

    assert_eq!(
        sql,
        [
            r#"SELECT "aspect" FROM "glyph" GROUP BY"#,
            r#"GROUPING SETS ("aspect", ("aspect", "image"), (), CUBE ("image"))"#,
        ]
        .join(" ")
    );
    // The outer set holds four members. The grammar gives `()` and `CUBE`
    // grouping-set nodes of their own, and leaves the bare expression a
    // column and the pair a row — which parse analysis, not the grammar,
    // reads as a set of one and a set of two.
    assert_eq!(grouping_sets(&sql), [(SETS, 4), (EMPTY, 0), (CUBE, 1)]);
    assert_eq!(parsed_nodes(&sql, "RowExpr").len(), 1);
}

// [spec:pgorm:req:sql.render.grouping+1/test]    a plain group-by list renders exactly as before,
// with no grouping-set node
#[test]
fn plain_group_by_is_unchanged() {
    let sql = Query::select()
        .column(Glyph::Aspect)
        .from(Glyph::Table)
        .group_by_col(Glyph::Aspect)
        .add_group_by([image().into()])
        .group_by_columns([(Glyph::Table, Glyph::Id)])
        .to_string();

    assert_eq!(
        sql,
        r#"SELECT "aspect" FROM "glyph" GROUP BY "aspect", "image", "glyph"."id""#
    );
    assert!(grouping_sets(&sql).is_empty());

    // A one-expression set is that expression.
    assert_eq!(
        grouped(GroupingElement::set([aspect()])),
        r#"SELECT "aspect" FROM "glyph" GROUP BY "aspect""#
    );
}

// [spec:pgorm:def:sql.ast.select.grouping/test]    an element joins the plain expressions in one
// list, in call order
#[test]
fn elements_and_expressions_share_one_list() {
    let sql = Query::select()
        .column(Glyph::Aspect)
        .from(Glyph::Table)
        .group_by_col(Glyph::Aspect)
        .group_by_element(GroupingElement::rollup([image()]))
        .add_group_by([Expr::col(Glyph::Id).into()])
        .to_string();

    assert_eq!(
        sql,
        r#"SELECT "aspect" FROM "glyph" GROUP BY "aspect", ROLLUP ("image"), "id""#
    );
    assert_eq!(grouping_sets(&sql), [(ROLLUP, 1)]);
}

// [spec:pgorm:def:sql.ast.select.grouping/test]    a tuple item is one unit of a ROLLUP or CUBE
// [spec:pgorm:req:sql.render.grouping+1/test]    an empty ROLLUP or CUBE renders `()`
#[test]
fn units_and_empty_lists() {
    let pair: SimpleExpr = Expr::tuple([aspect().into(), image().into()]).into();
    let sql = grouped(GroupingElement::rollup([pair, Expr::col(Glyph::Id).into()]));
    assert_eq!(
        sql,
        r#"SELECT "aspect" FROM "glyph" GROUP BY ROLLUP (("aspect", "image"), "id")"#
    );
    assert_eq!(grouping_sets(&sql), [(ROLLUP, 2)]);

    for element in [
        GroupingElement::rollup(Vec::<SimpleExpr>::new()),
        GroupingElement::cube(Vec::<SimpleExpr>::new()),
        GroupingElement::set(Vec::<SimpleExpr>::new()),
        GroupingElement::empty(),
    ] {
        let sql = grouped(element);
        assert_eq!(sql, r#"SELECT "aspect" FROM "glyph" GROUP BY ()"#);
        assert_eq!(grouping_sets(&sql), [(EMPTY, 0)]);
    }
}

// [spec:pgorm:def:sql.ast.func+12/test]    `Func::grouping` renders the GROUPING keyword, which the
// parser reads as its own node rather than as a call
#[test]
fn grouping_function_is_the_grouping_node() {
    let sql = Query::select()
        .expr(Func::grouping(aspect()).arg(image()))
        .from(Glyph::Table)
        .group_by_element(GroupingElement::cube([aspect(), image()]))
        .to_string();

    assert_eq!(
        sql,
        r#"SELECT GROUPING("aspect", "image") FROM "glyph" GROUP BY CUBE ("aspect", "image")"#
    );
    let grouping = parsed_nodes(&sql, "GroupingFunc");
    assert_eq!(grouping.len(), 1);
    assert_eq!(grouping[0]["args"].as_array().map(Vec::len), Some(2));
    assert!(parsed_nodes(&sql, "FuncCall").is_empty());

    // A function *named* grouping is a different thing: a call the parser
    // resolves against the catalog.
    let named = Query::select()
        .expr(Func::named(Name::runtime("grouping")).arg(aspect()))
        .from(Glyph::Table)
        .to_string();
    assert!(parsed_nodes(&named, "GroupingFunc").is_empty(), "{named}");
}
