use super::*;
use crate::oracle::{assert_eq, parsed_nodes};

/// libpg_query's `SubLinkType`, by the number it serialises as.
const ARRAY_SUBLINK: i64 = 7;

fn ids() -> SelectStatement {
    Query::select().column(Glyph::Id).from(Glyph::Table).take()
}

/// The kind of each sublink in `sql`'s parse tree, in tree order.
fn sublink_kinds(sql: &str) -> Vec<Option<i64>> {
    parsed_nodes(sql, "SubLink")
        .iter()
        .map(|sublink| sublink["sub_link_type"].as_i64())
        .collect()
}

// [spec:pgorm:def:sql.ast.expr+8/test]    `Expr::array` wraps a SELECT in `ARRAY(...)`, which
// the grammar reads as an ARRAY sublink, standing alone or as the array operand of `= ANY`
// [spec:pgorm:req:sql.render.subquery+5/test]    the ARRAY prefix directly before the
// parenthesized sub-statement
#[test]
fn an_array_subquery_renders_as_array_sublink() {
    let projected = Query::select()
        .expr_as(Expr::array(ids()), Name::runtime("ids"))
        .to_string();
    assert_eq!(
        projected,
        r#"SELECT ARRAY(SELECT "id" FROM "glyph") AS "ids""#
    );
    assert_eq!(sublink_kinds(&projected), [Some(ARRAY_SUBLINK)]);

    let compared = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .and_where(Expr::col(Glyph::Aspect).eq(Func::any(Expr::array(ids()))))
        .to_string();
    assert_eq!(
        compared,
        [
            r#"SELECT "id" FROM "glyph" WHERE "aspect""#,
            r#"= ANY(ARRAY(SELECT "id" FROM "glyph"))"#,
        ]
        .join(" ")
    );
    assert_eq!(sublink_kinds(&compared), [Some(ARRAY_SUBLINK)]);
}
