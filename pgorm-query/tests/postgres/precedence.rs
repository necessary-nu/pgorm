use super::*;
use crate::oracle::{assert_eq, assert_rejected, parsed_nodes};

/// An expression as the grammar grouped it: a boolean node as its operator
/// over its arguments, an operator expression as its operator name — over its
/// operands, a BETWEEN's two bounds among them, when any of those is not an
/// atom — and anything else as `_`.
fn grouping(node: &serde_json::Value) -> String {
    if let Some(boolean) = node.get("BoolExpr") {
        let operator = match boolean["boolop"].as_i64() {
            Some(1) => "AND",
            Some(2) => "OR",
            Some(3) => "NOT",
            other => panic!("no boolean operator {other:?}"),
        };
        format!("{operator}({})", groupings(&boolean["args"]).join(", "))
    } else if let Some(operation) = node.get("AExpr") {
        let name = operation["name"][0]["String"]["sval"]
            .as_str()
            .unwrap_or_default();
        let operands: Vec<String> = [&operation["lexpr"], &operation["rexpr"]]
            .into_iter()
            .flat_map(|operand| match operand.get("List") {
                Some(list) => groupings(&list["items"]),
                None => vec![grouping(operand)],
            })
            .collect();
        if operands.iter().all(|operand| operand == "_") {
            name.to_owned()
        } else {
            format!("{name}({})", operands.join(", "))
        }
    } else {
        "_".to_owned()
    }
}

fn groupings(nodes: &serde_json::Value) -> Vec<String> {
    nodes
        .as_array()
        .into_iter()
        .flatten()
        .map(grouping)
        .collect()
}

fn where_grouping(sql: &str) -> String {
    let select = parsed_nodes(sql, "SelectStmt")
        .into_iter()
        .next()
        .expect("a SELECT");
    grouping(&select["where_clause"])
}

/// The operators outside the comparison, BETWEEN, IN, LIKE and IS families
/// that answer a boolean, each with its lexeme.
const BOOLEAN_OPERATORS: [(BinOper, &str); 18] = [
    (BinOper::Contains, "@>"),
    (BinOper::Contained, "<@"),
    (BinOper::Overlap, "&&"),
    (BinOper::Matches, "@@"),
    (BinOper::Similarity, "%"),
    (BinOper::WordSimilarity, "<%"),
    (BinOper::StrictWordSimilarity, "<<%"),
    (BinOper::HasJsonKey, "?"),
    (BinOper::HasAnyJsonKeys, "?|"),
    (BinOper::HasAllJsonKeys, "?&"),
    (BinOper::JsonPathExists, "@?"),
    (BinOper::Regex, "~"),
    (BinOper::RegexCaseInsensitive, "~*"),
    (BinOper::NotRegex, "!~"),
    (BinOper::NotRegexCaseInsensitive, "!~*"),
    (BinOper::NotExtendRightOf, "&<"),
    (BinOper::NotExtendLeftOf, "&>"),
    (BinOper::AdjacentTo, "-|-"),
];

/// Each projected expression of a SELECT as the grammar grouped it.
fn select_groupings(sql: &str) -> Vec<String> {
    let select = parsed_nodes(sql, "SelectStmt")
        .into_iter()
        .next()
        .expect("a SELECT");
    select["target_list"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|target| grouping(&target["ResTarget"]["val"]))
        .collect()
}

// [spec:pgorm:def:sql.render.precedence+14/test]    every boolean-valued operator outside the
// comparison, BETWEEN, IN, LIKE and IS families renders bare under AND, OR and NOT, and the grammar
// groups the bare render as the condition was built
#[test]
fn boolean_operators_render_bare_under_logic() {
    for (operator, lexeme) in BOOLEAN_OPERATORS {
        let test = |column: Char, value: i32| Expr::col(column).binary(operator, value);
        let sql = Query::select()
            .column(Char::Id)
            .from(Char::Table)
            .cond_where(
                Condition::all()
                    .add(test(Char::Character, 1))
                    .add(
                        Condition::any()
                            .add(test(Char::SizeW, 2))
                            .add(test(Char::SizeH, 3)),
                    )
                    .add(test(Char::FontId, 4).not()),
            )
            .to_string();
        assert_eq!(
            sql,
            format!(
                r#"SELECT "id" FROM "character" WHERE "character" {lexeme} 1 AND ("size_w" {lexeme} 2 OR "size_h" {lexeme} 3) AND (NOT "font_id" {lexeme} 4)"#
            )
        );
        assert_eq!(
            where_grouping(&sql),
            format!("AND({lexeme}, OR({lexeme}, {lexeme}), NOT({lexeme}))")
        );
    }
}

// [spec:pgorm:def:sql.render.precedence+14/test]    a boolean generic operator renders bare as
// either operand of a comparison, IS DISTINCT FROM, BETWEEN, IN and LIKE, and the grammar groups
// each bare render as the expression was built
// [spec:pgorm:req:sql.render.parens+5/test]    a boolean generic operator is a BETWEEN bound the
// grammar ranks above BETWEEN, so it renders bare as either bound
#[test]
fn boolean_operators_render_bare_under_comparisons() {
    let truth = || SimpleExpr::Constant(true.into());
    for (operator, lexeme) in BOOLEAN_OPERATORS {
        let test = |column: Char, value: i32| Expr::col(column).binary(operator, value);
        let sql = Query::select()
            .expr(test(Char::Character, 1).binary(BinOper::Equal, truth()))
            .expr(truth().binary(BinOper::NotEqual, test(Char::SizeW, 2)))
            .expr(Expr::expr(test(Char::SizeH, 3)).is_distinct_from(truth()))
            .expr(Expr::expr(truth()).is_not_distinct_from(test(Char::FontId, 4)))
            .expr(Expr::expr(test(Char::Ascii, 5)).is_in([truth()]))
            .expr(Expr::expr(test(Char::Character, 6)).between(false, true))
            .expr(Expr::expr(truth()).between(test(Char::SizeW, 7), test(Char::SizeH, 8)))
            .expr(test(Char::FontId, 9).binary(BinOper::Like, Expr::col(Char::Ascii)))
            .expr(Expr::col(Char::Ascii).binary(BinOper::NotILike, test(Char::Character, 10)))
            .from(Char::Table)
            .to_string();
        assert_eq!(
            sql,
            [
                format!(r#"SELECT "character" {lexeme} 1 = TRUE"#),
                format!(r#"TRUE <> "size_w" {lexeme} 2"#),
                format!(r#""size_h" {lexeme} 3 IS DISTINCT FROM TRUE"#),
                format!(r#"TRUE IS NOT DISTINCT FROM "font_id" {lexeme} 4"#),
                format!(r#""ascii" {lexeme} 5 IN (TRUE)"#),
                format!(r#""character" {lexeme} 6 BETWEEN FALSE AND TRUE"#),
                format!(r#"TRUE BETWEEN "size_w" {lexeme} 7 AND "size_h" {lexeme} 8"#),
                format!(r#""font_id" {lexeme} 9 LIKE "ascii""#),
                format!(r#""ascii" NOT ILIKE "character" {lexeme} 10 FROM "character""#),
            ]
            .join(", ")
        );
        assert_eq!(
            select_groupings(&sql),
            [
                format!("=({lexeme}, _)"),
                format!("<>(_, {lexeme})"),
                format!("=({lexeme}, _)"),
                format!("=(_, {lexeme})"),
                format!("=({lexeme}, _)"),
                format!("BETWEEN({lexeme}, _, _)"),
                format!("BETWEEN(_, {lexeme}, {lexeme})"),
                format!("~~({lexeme}, _)"),
                format!("!~~*(_, {lexeme})"),
            ]
        );
    }
}

// [spec:pgorm:def:sql.render.precedence+14/test]    the BETWEEN family renders bare under AND, OR
// and NOT, negated and symmetric forms included, as its left and its right operand, and the
// grammar takes each AND after a lower bound as BETWEEN's own and the next as the logical one
#[test]
fn between_family_renders_bare_under_logic() {
    let upper: SimpleExpr = Expr::col(Char::SizeH).add(1);
    let sql = Query::select()
        .column(Char::Id)
        .from(Char::Table)
        .cond_where(
            Condition::all()
                .add(Expr::col(Char::Character).between(1, 2))
                .add(
                    Condition::any()
                        .add(Expr::col(Char::SizeW).not_between(3, 4))
                        .add(Expr::col(Char::SizeH).between_symmetric(6, 5)),
                )
                .add(Expr::col(Char::FontId).not_between_symmetric(8, 7).not())
                .add(Expr::col(Char::Ascii).between(Expr::val(9), upper)),
        )
        .to_string();
    assert_eq!(
        sql,
        r#"SELECT "id" FROM "character" WHERE "character" BETWEEN 1 AND 2 AND ("size_w" NOT BETWEEN 3 AND 4 OR "size_h" BETWEEN SYMMETRIC 6 AND 5) AND (NOT "font_id" NOT BETWEEN SYMMETRIC 8 AND 7) AND "ascii" BETWEEN 9 AND "size_h" + 1"#
    );
    assert_eq!(
        where_grouping(&sql),
        "AND(BETWEEN, OR(NOT BETWEEN, BETWEEN SYMMETRIC), NOT(NOT BETWEEN SYMMETRIC), BETWEEN(_, _, +))"
    );
}

// [spec:pgorm:req:sql.render.parens+5/test]    a BETWEEN's bounds are its own operands rather than
// an AND's: an arithmetic bound renders bare, and a comparison, LIKE or BETWEEN bound keeps the
// parentheses without which the grammar regroups the render or refuses it
#[test]
fn between_bounds_are_operands_of_between() {
    let sql = Query::select()
        .column(Char::Id)
        .from(Char::Table)
        .and_where(
            Expr::col(Char::Character)
                .between(Expr::col(Char::SizeW).add(1), Expr::col(Char::SizeH).mul(2)),
        )
        .and_where(
            Expr::col(Char::Ascii)
                .between(Expr::col(Char::SizeW).eq(1), Expr::col(Char::SizeH).eq(2)),
        )
        .and_where(Expr::col(Char::Ascii).not_between(
            Expr::col(Char::Character).like("a%"),
            Expr::col(Char::FontId).between(3, 4),
        ))
        .to_string();
    assert_eq!(
        sql,
        r#"SELECT "id" FROM "character" WHERE "character" BETWEEN "size_w" + 1 AND "size_h" * 2 AND "ascii" BETWEEN ("size_w" = 1) AND ("size_h" = 2) AND "ascii" NOT BETWEEN ("character" LIKE 'a%') AND ("font_id" BETWEEN 3 AND 4)"#
    );
    assert_eq!(
        where_grouping(&sql),
        "AND(BETWEEN(_, +, *), BETWEEN(_, =, =), NOT BETWEEN(_, ~~, BETWEEN))"
    );

    let bare =
        r#"SELECT "id" FROM "character" WHERE "ascii" BETWEEN "size_w" = 1 AND "size_h" = 2"#;
    assert_eq!(where_grouping(bare), "=(BETWEEN(_, =, _), _)");
    assert_rejected(
        r#"SELECT "id" FROM "character" WHERE "ascii" NOT BETWEEN "character" LIKE 'a%' AND TRUE"#,
    );
}
