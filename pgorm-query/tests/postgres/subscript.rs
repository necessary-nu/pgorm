use super::*;
use crate::oracle::{assert_eq, parsed_nodes};

fn tags() -> Expr {
    Expr::col(Name::runtime("tags"))
}

fn select(expr: impl Into<SimpleExpr>) -> String {
    Query::select().expr(expr).from(Glyph::Table).to_string()
}

/// The one `A_Indirection` in `sql` — the node PostgreSQL's parser builds for
/// a subscripted expression — as the parser read it.
fn indirection(sql: &str) -> serde_json::Value {
    let mut found = parsed_nodes(sql, "AIndirection");
    assert_eq!(
        found.len(),
        1,
        "exactly one subscripted expression in {sql}"
    );
    found.remove(0)
}

/// Each subscript of an indirection as `(is_slice, lower, upper)`, a bound
/// read as the integer literal it holds.
fn subscripts(indirection: &serde_json::Value) -> Vec<(bool, Option<i64>, Option<i64>)> {
    let bound = |node: &serde_json::Value| node["AConst"]["val"]["Ival"]["ival"].as_i64();
    indirection["indirection"]
        .as_array()
        .expect("an indirection list")
        .iter()
        .map(|item| {
            let indices = &item["AIndices"];
            (
                indices["is_slice"].as_bool().unwrap_or(false),
                indices.get("lidx").and_then(bound),
                indices.get("uidx").and_then(bound),
            )
        })
        .collect()
}

// [spec:pgorm:req:sql.ast.expr.subscript+2/test]    `index` builds an element subscript
// [spec:pgorm:req:sql.render.subscript/test]    a column takes the subscript bare
#[test]
fn an_index_subscripts_a_column_bare() {
    let sql = select(tags().index(1));

    assert_eq!(sql, r#"SELECT "tags"[1] FROM "glyph""#);
    let node = indirection(&sql);
    assert_eq!(
        node["arg"]["ColumnRef"]["fields"][0]["String"]["sval"],
        "tags"
    );
    assert_eq!(subscripts(&node), [(false, None, Some(1))]);

    // A qualified column takes it bare too.
    let sql = select(Expr::col((Glyph::Table, Glyph::Tokens)).index(2));
    assert_eq!(sql, r#"SELECT "glyph"."tokens"[2] FROM "glyph""#);
    assert_eq!(subscripts(&indirection(&sql)), [(false, None, Some(2))]);
}

// [spec:pgorm:req:sql.ast.expr.subscript+2/test]    `slice`, `slice_from`, `slice_to` and the
// general `subscript` cover every pair of optional bounds
// [spec:pgorm:req:sql.render.subscript/test]    `[l:u]`, `[l:]`, `[:u]`, `[:]`
#[test]
fn slices_render_each_pair_of_bounds() {
    let cases = [
        (
            tags().slice(2, 3),
            r#""tags"[2:3]"#,
            (true, Some(2), Some(3)),
        ),
        (tags().slice_from(2), r#""tags"[2:]"#, (true, Some(2), None)),
        (tags().slice_to(3), r#""tags"[:3]"#, (true, None, Some(3))),
        (
            tags().subscript(Subscript::Slice(None, None)),
            r#""tags"[:]"#,
            (true, None, None),
        ),
    ];

    for (expr, rendered, parsed) in cases {
        let sql = select(expr);
        assert_eq!(sql, format!(r#"SELECT {rendered} FROM "glyph""#));
        assert_eq!(subscripts(&indirection(&sql)), [parsed], "{sql}");
    }
}

// [spec:pgorm:req:sql.ast.expr.subscript+2/test]    chained subscripts are one multi-dimensional
// access
// [spec:pgorm:req:sql.render.subscript/test]    a subscripted base is not parenthesised, so the
// parser reads one indirection holding both subscripts
#[test]
fn chained_subscripts_are_one_indirection() {
    let sql = select(tags().index(1).index(2));

    assert_eq!(sql, r#"SELECT "tags"[1][2] FROM "glyph""#);
    assert_eq!(
        subscripts(&indirection(&sql)),
        [(false, None, Some(1)), (false, None, Some(2))]
    );

    let sql = select(tags().slice(1, 2).index(3).slice_from(4));
    assert_eq!(sql, r#"SELECT "tags"[1:2][3][4:] FROM "glyph""#);
    assert_eq!(
        subscripts(&indirection(&sql)),
        [
            (true, Some(1), Some(2)),
            (false, None, Some(3)),
            (true, Some(4), None)
        ]
    );
}

// [spec:pgorm:req:sql.render.subscript/test]    a function call, a cast, a value, a CASE and a
// subquery are parenthesised before the subscript
#[test]
fn computed_bases_are_parenthesised() {
    let split = Func::named(Name::runtime("string_to_array"))
        .arg(Expr::col(Glyph::Image))
        .arg(",");
    let cast = Expr::val("{1,2}").cast_as_type(TypeName::new(Name::runtime("int4")).array());
    let cases: [(SimpleExpr, &str); 5] = [
        (split.into(), r#"(string_to_array("image", ','))[1]"#),
        (cast, r#"(CAST('{1,2}' AS int4[]))[1]"#),
        (Expr::value(vec![1, 2]), r#"(ARRAY [1,2])[1]"#),
        (
            Expr::case_of(Expr::col(Glyph::Aspect))
                .when(1, Expr::col(Glyph::Tokens))
                .into(),
            r#"((CASE "aspect" WHEN 1 THEN "tokens" END))[1]"#,
        ),
        (
            SimpleExpr::SubQuery(
                None,
                Box::new(
                    Query::select()
                        .column(Glyph::Tokens)
                        .from(Glyph::Table)
                        .take()
                        .into_sub_query_statement(),
                ),
            ),
            r#"((SELECT "tokens" FROM "glyph"))[1]"#,
        ),
    ];

    for (base, rendered) in cases {
        let sql = select(Expr::expr(base).index(1));
        assert_eq!(sql, format!(r#"SELECT {rendered} FROM "glyph""#));
        assert_eq!(subscripts(&indirection(&sql)), [(false, None, Some(1))]);
    }
}

// [spec:pgorm:req:sql.render.subscript/test]    the unparenthesised forms the rule avoids are
// ones the grammar rejects
#[test]
fn unparenthesised_computed_bases_do_not_parse() {
    for rejected in [
        r#"SELECT string_to_array("image", ',')[1] FROM "glyph""#,
        r#"SELECT CAST('{1,2}' AS int4[])[1] FROM "glyph""#,
        r#"SELECT ARRAY [1,2][1] FROM "glyph""#,
    ] {
        assert!(crate::oracle::parses(rejected).is_err(), "{rejected}");
    }
}

// [spec:pgorm:req:sql.render.subscript/test]    parameters number in textual order: the base,
// then each bound
// [spec:pgorm:def:sql.render.precedence+14/test]    a subscript is an atom under an operator
#[test]
fn subscripts_bind_and_compose_as_atoms() {
    let array = Expr::val(vec![1, 2, 3]).cast_as_type(TypeName::new(Name::runtime("int4")).array());
    let (sql, values) = Query::select()
        .expr(Expr::expr(array).index(2).add(1))
        .from(Glyph::Table)
        .and_where(tags().slice(Expr::col(Glyph::Aspect), 3).is_not_null())
        .and_where(tags().index(1).eq(Expr::col(Glyph::Image)))
        .build();

    assert_eq!(
        sql,
        [
            r#"SELECT (CAST($1::int4[] AS int4[]))[$2] + $3 FROM "glyph""#,
            r#"WHERE "tags"["aspect":$4] IS NOT NULL AND "tags"[$5] = "image""#,
        ]
        .join(" ")
    );
    assert_eq!(
        values,
        Values(vec![
            vec![1, 2, 3].into(),
            2.into(),
            1.into(),
            3.into(),
            1.into(),
        ])
    );
    assert_eq!(parsed_nodes(&sql, "AIndirection").len(), 3);
}
