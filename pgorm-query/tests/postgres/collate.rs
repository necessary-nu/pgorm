use super::*;
use crate::oracle::{assert_eq, parsed_nodes};
use pg_query::protobuf::SortByDir;

fn image() -> Expr {
    Expr::col(Glyph::Image)
}

fn c() -> Name {
    Name::runtime("C")
}

fn select(expr: impl Into<SimpleExpr>) -> String {
    Query::select().expr(expr).from(Glyph::Table).to_string()
}

/// The one `CollateClause` in `sql` — the node PostgreSQL's parser builds for
/// an expression under a collation — as the parser read it.
fn collate_clause(sql: &str) -> serde_json::Value {
    let mut found = parsed_nodes(sql, "CollateClause");
    assert_eq!(found.len(), 1, "exactly one COLLATE in {sql}");
    found.remove(0)
}

/// A collation name's parts, as the parser read them.
fn collname(clause: &serde_json::Value) -> Vec<String> {
    clause["collname"]
        .as_array()
        .expect("a collation name")
        .iter()
        .map(|part| {
            part["String"]["sval"]
                .as_str()
                .expect("a name part")
                .to_owned()
        })
        .collect()
}

// [spec:pgorm:req:sql.ast.expr.collate/test]    `collate` puts an expression under a named
// collation, bare or schema-qualified
// [spec:pgorm:req:sql.render.collate/test]    the clause renders inside its own parentheses,
// each part of the name quoted
#[test]
fn a_collation_renders_quoted_after_its_operand() {
    let cases = [
        (image().collate(c()), r#"("image" COLLATE "C")"#, vec!["C"]),
        (
            image().collate(Name::runtime("und-x-icu")),
            r#"("image" COLLATE "und-x-icu")"#,
            vec!["und-x-icu"],
        ),
        (
            image().collate((Name::runtime("pg_catalog"), Name::runtime("default"))),
            r#"("image" COLLATE "pg_catalog"."default")"#,
            vec!["pg_catalog", "default"],
        ),
    ];

    for (expr, rendered, parts) in cases {
        let sql = select(expr);
        assert_eq!(sql, format!(r#"SELECT {rendered} FROM "glyph""#));
        let clause = collate_clause(&sql);
        assert_eq!(collname(&clause), parts, "{sql}");
        assert_eq!(
            clause["arg"]["ColumnRef"]["fields"][0]["String"]["sval"], "image",
            "{sql}"
        );
    }
}

// [spec:pgorm:req:sql.render.collate/test]    an operand that binds looser than COLLATE is
// parenthesised, so the clause collates all of it; an atom is written bare
#[test]
fn a_compound_operand_is_parenthesised() {
    let joined = Expr::expr(image().concat(Expr::col(Glyph::Aspect))).collate(c());
    let sql = select(joined);
    assert_eq!(
        sql,
        r#"SELECT (("image" || "aspect") COLLATE "C") FROM "glyph""#
    );
    let clause = collate_clause(&sql);
    assert_eq!(clause["arg"]["AExpr"]["name"][0]["String"]["sval"], "||");

    let lowered = Expr::expr(Func::lower(image())).collate(c());
    let sql = select(lowered);
    assert_eq!(sql, r#"SELECT (LOWER("image") COLLATE "C") FROM "glyph""#);
    assert!(collate_clause(&sql)["arg"].get("FuncCall").is_some());

    // A collated expression can itself be collated again; the outer clause
    // wins, and the inner one is already an atom.
    let twice = image().collate(c()).collate(Name::runtime("POSIX"));
    let sql = select(twice);
    assert_eq!(
        sql,
        r#"SELECT (("image" COLLATE "C") COLLATE "POSIX") FROM "glyph""#
    );
    assert_eq!(parsed_nodes(&sql, "CollateClause").len(), 2);
}

// [spec:pgorm:req:sql.render.collate/test]    a collated operand is an atom under every
// operator, including the two `b_expr` positions that have no COLLATE of their own
// [spec:pgorm:def:sql.render.precedence+14/test]
#[test]
fn a_collated_operand_is_an_atom() {
    let sql = select(image().collate(c()).lt("b"));
    assert_eq!(sql, r#"SELECT ("image" COLLATE "C") < 'b' FROM "glyph""#);
    let compared = &parsed_nodes(&sql, "AExpr")[0];
    assert!(compared["lexpr"].get("CollateClause").is_some(), "{sql}");

    // BETWEEN's lower bound is a `b_expr`, which has no COLLATE: bare, the
    // clause would be a syntax error there.
    let sql = select(image().between(Expr::val("a").collate(c()), Expr::val("c")));
    assert_eq!(
        sql,
        r#"SELECT "image" BETWEEN ('a' COLLATE "C") AND 'c' FROM "glyph""#
    );
    let range = &parsed_nodes(&sql, "AExpr")[0];
    assert!(
        range["rexpr"]["List"]["items"][0]
            .get("CollateClause")
            .is_some(),
        "{sql}"
    );
}

// [spec:pgorm:req:sql.render.collate/test]    a collated DEFAULT stays the default's: bare,
// `DEFAULT 'a' COLLATE "C"` parses as the column's own collation clause
// [spec:pgorm:req:sql.ddl.column-def+13/test]
#[test]
fn a_collated_default_stays_the_default() {
    let sql = Table::create(Glyph::Table)
        .col(
            ColumnDef::new(Glyph::Image)
                .text()
                .default(Expr::val("a").collate(c())),
        )
        .to_string();
    assert_eq!(
        sql,
        r#"CREATE TABLE "glyph" ( "image" text DEFAULT ('a' COLLATE "C") )"#
    );
    let column = &parsed_nodes(&sql, "ColumnDef")[0];
    assert!(column.get("coll_clause").is_none(), "{sql}");
    let default = &parsed_nodes(&sql, "Constraint")[0];
    assert!(default["raw_expr"].get("CollateClause").is_some(), "{sql}");
}

// [spec:pgorm:req:sql.ast.expr.collate/test]    ORDER BY takes a collation through its sort
// key's expression, as the grammar does
// [spec:pgorm:req:sql.render.collate/test]
#[test]
fn an_ordering_takes_a_collated_expression() {
    let sql = Query::select()
        .column(Glyph::Image)
        .from(Glyph::Table)
        .order_by_expr_with_nulls(image().collate(c()).into(), Order::Desc, NullOrdering::Last)
        .to_string();
    assert_eq!(
        sql,
        r#"SELECT "image" FROM "glyph" ORDER BY ("image" COLLATE "C") DESC NULLS LAST"#
    );
    let sort = &parsed_nodes(&sql, "SortBy")[0];
    assert!(sort["node"].get("CollateClause").is_some(), "{sql}");
    assert_eq!(sort["sortby_dir"], SortByDir::SortbyDesc as i32);
    assert_eq!(collname(&collate_clause(&sql)), ["C"]);
}

// [spec:pgorm:req:sql.render.collate/test]    a bound operand is a placeholder, and the
// collation stays in the text
#[test]
fn a_bound_operand_keeps_its_placeholder() {
    let (sql, values) = Query::select().expr(Expr::val("b").collate(c())).build();
    assert_eq!(sql, r#"SELECT ($1 COLLATE "C")"#);
    assert_eq!(values.0, [Value::from("b")]);
    assert!(collate_clause(&sql)["arg"].get("ParamRef").is_some());
}

// [spec:pgorm:req:sql.ddl.column-def+13/test]    a column's collation follows its type, and a
// second call replaces the first
#[test]
fn a_column_declares_its_collation() {
    let sql = Table::create(Glyph::Table)
        .col(
            ColumnDef::new(Glyph::Image)
                .text()
                .not_null()
                .collate(Name::runtime("POSIX"))
                .collate((Name::runtime("pg_catalog"), c())),
        )
        .col(ColumnDef::new(Glyph::Aspect).text())
        .to_string();
    assert_eq!(
        sql,
        [
            r#"CREATE TABLE "glyph" ("#,
            r#""image" text COLLATE "pg_catalog"."C" NOT NULL,"#,
            r#""aspect" text"#,
            r#")"#,
        ]
        .join(" ")
    );
    let columns = parsed_nodes(&sql, "ColumnDef");
    assert_eq!(collname(&columns[0]["coll_clause"]), ["pg_catalog", "C"]);
    assert!(columns[1].get("coll_clause").is_none());

    let column = ColumnDef::new(Glyph::Image).text().collate(c()).to_owned();
    assert_eq!(
        column.get_collation().map(|c| c.name().to_string()),
        Some("C".to_owned())
    );
    assert!(column.get_collation().and_then(Collation::schema).is_none());
}

// [spec:pgorm:req:sql.ddl.alter-table+14/test]    ADD COLUMN spells the collation as CREATE TABLE
// does, and a modified column carries it on the retype
// [spec:pgorm:req:sql.ddl.column-def+13/test]
#[test]
fn an_altered_column_carries_its_collation() {
    let sql = Table::alter(Glyph::Table)
        .add_column(ColumnDef::new(Glyph::Image).text().collate(c()))
        .modify_column(
            ColumnDef::new(Glyph::Aspect)
                .text()
                .collate(Name::runtime("POSIX"))
                .not_null(),
        )
        .to_string();
    assert_eq!(
        sql,
        [
            r#"ALTER TABLE "glyph""#,
            r#"ADD COLUMN "image" text COLLATE "C","#,
            r#"ALTER COLUMN "aspect" TYPE text COLLATE "POSIX","#,
            r#"ALTER COLUMN "aspect" SET NOT NULL"#,
        ]
        .join(" ")
    );
    let columns = parsed_nodes(&sql, "ColumnDef");
    assert_eq!(collname(&columns[0]["coll_clause"]), ["C"]);
    assert_eq!(collname(&columns[1]["coll_clause"]), ["POSIX"]);

    // Without a type there is no retype for the collation to ride on.
    assert_eq!(
        Table::alter(Glyph::Table)
            .modify_column(ColumnDef::new(Glyph::Aspect).collate(c()).not_null())
            .to_string(),
        r#"ALTER TABLE "glyph" ALTER COLUMN "aspect" SET NOT NULL"#
    );
}
