use super::*;
use crate::oracle::{assert_eq, parsed_nodes};

/// The one `CaseExpr` in `sql`, as the parser read it.
fn case_expr(sql: &str) -> serde_json::Value {
    let mut found = parsed_nodes(sql, "CaseExpr");
    assert_eq!(found.len(), 1, "exactly one CASE in {sql}");
    found.remove(0)
}

fn aspect_names() -> SimpleCaseStatement {
    Expr::case_of(Expr::col(Glyph::Aspect))
        .when(1, "one")
        .when(2, "two")
        .finally("many")
}

// [spec:pgorm:def:sql.ast.case+3/test]    `Expr::case_of` takes the operand, `when` the arms,
// `finally` the ELSE
// [spec:pgorm:req:sql.render.case/test]    the operand once after `CASE`, each value bare
// between `WHEN` and `THEN`
#[test]
fn simple_case_writes_its_operand_once() {
    let sql = Query::select()
        .expr_as(aspect_names(), Name::runtime("name"))
        .from(Glyph::Table)
        .to_string();

    assert_eq!(
        sql,
        r#"SELECT (CASE "aspect" WHEN 1 THEN 'one' WHEN 2 THEN 'two' ELSE 'many' END) AS "name" FROM "glyph""#
    );

    // The parser reads the operand into `arg` — the field that makes a CASE
    // the simple form — and each arm's value as a bare constant, not as a
    // condition over the operand.
    let case = case_expr(&sql);
    assert_eq!(
        case["arg"]["ColumnRef"]["fields"][0]["String"]["sval"],
        "aspect"
    );
    let arms = case["args"].as_array().expect("arms");
    assert_eq!(arms.len(), 2);
    assert_eq!(
        arms[0]["CaseWhen"]["expr"]["AConst"]["val"]["Ival"]["ival"],
        1
    );
    assert_eq!(
        arms[1]["CaseWhen"]["expr"]["AConst"]["val"]["Ival"]["ival"],
        2
    );
    assert_eq!(
        case["defresult"]["AConst"]["val"]["Sval"]["sval"], "many",
        "ELSE is the default result"
    );
}

// [spec:pgorm:req:sql.render.case/test]    the searched form carries no operand, and the
// parser tells the two apart by exactly that
#[test]
fn searched_case_leaves_the_operand_empty() {
    let sql = Query::select()
        .expr(Expr::case(Expr::col(Glyph::Aspect).eq(1), "one").finally("many"))
        .from(Glyph::Table)
        .to_string();

    assert_eq!(
        sql,
        r#"SELECT (CASE WHEN ("aspect" = 1) THEN 'one' ELSE 'many' END) FROM "glyph""#
    );
    assert!(case_expr(&sql).get("arg").is_none(), "searched form: {sql}");
}

// [spec:pgorm:def:sql.ast.case+3/test]    `get_arms` reads each arm back in order as the
// expression its condition means and its result, a one-expression condition as that
// expression and a `Condition` tree joined and negated as it renders, and `get_else` the ELSE
#[test]
fn searched_case_reads_back_its_arms() {
    let positive = Expr::col(Glyph::Aspect).gt(0);
    let negative = Expr::col(Glyph::Aspect).lt(0);
    let large = Expr::col(Glyph::Id).gt(9);
    let case = Expr::case(positive.clone(), "positive")
        .case(
            Condition::any()
                .add(negative.clone())
                .add(large.clone())
                .not(),
            "small",
        )
        .finally("zero");

    let arms: Vec<(SimpleExpr, SimpleExpr)> = case
        .get_arms()
        .map(|(condition, result)| (condition, result.clone()))
        .collect();
    assert_eq!(
        arms,
        vec![
            (positive, "positive".into()),
            (negative.or(large).not(), "small".into()),
        ]
    );
    assert_eq!(case.get_else(), Some(&"zero".into()));
    assert_eq!(Expr::case(Expr::col(Glyph::Id).eq(1), 1).get_else(), None);
}

// [spec:pgorm:def:sql.ast.case+3/test]    without `finally` there is no ELSE, and an unmatched
// operand yields NULL
#[test]
fn simple_case_without_else_has_no_default() {
    let sql = Query::select()
        .expr(Expr::case_of(Expr::col(Glyph::Aspect)).when(1, "one"))
        .from(Glyph::Table)
        .to_string();

    assert_eq!(
        sql,
        r#"SELECT (CASE "aspect" WHEN 1 THEN 'one' END) FROM "glyph""#
    );
    let case = case_expr(&sql);
    assert!(case.get("arg").is_some());
    assert!(case.get("defresult").is_none());
}

// [spec:pgorm:req:sql.render.case/test]    parameters number in textual order: operand, then
// each arm's value and result, then the ELSE
#[test]
fn simple_case_binds_in_textual_order() {
    let (sql, values) = Query::select()
        .expr(
            Expr::case_of(Expr::val(7))
                .when(1, "one")
                .when(7, "seven")
                .finally("other"),
        )
        .build();

    assert_eq!(
        sql,
        r#"SELECT (CASE $1 WHEN $2 THEN $3 WHEN $4 THEN $5 ELSE $6 END)"#
    );
    assert_eq!(
        values,
        Values(vec![
            7.into(),
            1.into(),
            "one".into(),
            7.into(),
            "seven".into(),
            "other".into(),
        ])
    );
}

// [spec:pgorm:req:sql.render.case/test]    a compound operand or value needs no parentheses:
// the keywords delimit it
// [spec:pgorm:def:sql.render.precedence+14/test]    the simple form is an atom under an
// operator
#[test]
fn simple_case_operands_render_bare() {
    let sql = Query::select()
        .expr(
            Expr::case_of(Expr::col(Glyph::Aspect).add(1))
                .when(Expr::col(Glyph::Id).mul(2), Expr::col(Glyph::Image))
                .when(
                    Expr::col(Glyph::Id).gt(1).and(Expr::col(Glyph::Id).lt(9)),
                    "boolean arm",
                ),
        )
        .from(Glyph::Table)
        .to_string();

    assert_eq!(
        sql,
        r#"SELECT (CASE "aspect" + 1 WHEN "id" * 2 THEN "image" WHEN "id" > 1 AND "id" < 9 THEN 'boolean arm' END) FROM "glyph""#
    );
    let case = case_expr(&sql);
    assert_eq!(case["arg"]["AExpr"]["name"][0]["String"]["sval"], "+");
    assert_eq!(
        case["args"][0]["CaseWhen"]["expr"]["AExpr"]["name"][0]["String"]["sval"],
        "*"
    );
    assert_eq!(
        case["args"][1]["CaseWhen"]["expr"]["BoolExpr"]["boolop"], 1,
        "AND_EXPR"
    );

    // Under an operator it keeps only its own parentheses.
    let sql = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .and_where(
            Expr::expr(aspect_names())
                .eq("one")
                .and(Expr::expr(aspect_names()).ne("two").not()),
        )
        .to_string();
    assert_eq!(
        sql,
        [
            r#"SELECT "id" FROM "glyph" WHERE"#,
            r#"(CASE "aspect" WHEN 1 THEN 'one' WHEN 2 THEN 'two' ELSE 'many' END) = 'one'"#,
            r#"AND (NOT (CASE "aspect" WHEN 1 THEN 'one' WHEN 2 THEN 'two' ELSE 'many' END) <> 'two')"#,
        ]
        .join(" ")
    );
    assert_eq!(parsed_nodes(&sql, "CaseExpr").len(), 2);
}

// [spec:pgorm:def:sql.ast.case+3/test]    the two forms nest in each other's results
#[test]
fn case_forms_nest_in_each_other() {
    let sql = Query::select()
        .expr(
            Expr::case_of(Expr::col(Glyph::Aspect))
                .when(
                    1,
                    Expr::case(Expr::col(Glyph::Image).is_null(), "blank").finally(
                        Expr::case_of(Expr::col(Glyph::Image))
                            .when("a", "first")
                            .finally("later"),
                    ),
                )
                .finally("other"),
        )
        .from(Glyph::Table)
        .to_string();

    assert_eq!(
        sql,
        [
            r#"SELECT (CASE "aspect" WHEN 1 THEN"#,
            r#"(CASE WHEN ("image" IS NULL) THEN 'blank' ELSE"#,
            r#"(CASE "image" WHEN 'a' THEN 'first' ELSE 'later' END) END)"#,
            r#"ELSE 'other' END) FROM "glyph""#,
        ]
        .join(" ")
    );
    let cases = parsed_nodes(&sql, "CaseExpr");
    let operands: Vec<bool> = cases.iter().map(|case| case.get("arg").is_some()).collect();
    assert_eq!(operands, [true, false, true], "simple, searched, simple");
}
