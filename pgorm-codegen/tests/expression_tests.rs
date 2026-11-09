//! A column's `DEFAULT` and generation expression: read from DDL into the
//! subset codegen holds, carried on the bridged statement, and written into
//! the entity as the Rust that builds the same expression back.

mod common;

use common::runtime_name as n;
use common::*;
use pgorm_codegen::parse_schema;
use pgorm_query::{
    ColumnDef, ColumnSpec, Expr, ExprTrait, Func, GeneratedKind, Query, TableCreateStatement,
};

const SCHEMA: &str = include_str!("sql/expressions/schema.sql");

/// The entities the bridge generates from `SCHEMA`, compiled here as well as
/// compared: the derive accepts every expression the writer prints.
#[path = "sql/expressions/formula.rs"]
mod formula;
#[path = "sql/expressions/stock_code.rs"]
mod stock_code;
#[path = "sql/expressions/stock_item.rs"]
mod stock_item;

/// Each column's `DEFAULT` or generation expression, as the column, the
/// kind and the expression rendered.
fn expressions(table: &TableCreateStatement) -> Vec<String> {
    let mut found = Vec::new();
    for column in table.get_columns() {
        for spec in column.get_column_spec() {
            let (kind, expr) = match spec {
                ColumnSpec::Default(expr) => ("DEFAULT", expr),
                ColumnSpec::Generated {
                    expr,
                    kind: GeneratedKind::Stored,
                } => ("STORED", expr),
                ColumnSpec::Generated {
                    expr,
                    kind: GeneratedKind::Virtual,
                } => ("VIRTUAL", expr),
                _ => continue,
            };
            let rendered = Query::select().expr(expr.clone()).to_string();
            found.push(format!("{} {kind} {rendered}", column.get_column_name()));
        }
    }
    found
}

// [spec:pgorm:sem:codegen.ddl.tables+13/test]    a column's DEFAULT and generation expression,
// stored or virtual, ride on the bridged statement
// [spec:pgorm:sem:codegen.entity.expressions+3/test]    and reach the entity as the Rust that
// builds the same expressions: the schema built from the generated entities carries each
// expression the bridge read, rendered alike
// [spec:pgorm:sem:codegen.entity.compact.attrs+14/test]
#[test]
fn schema_expressions_reach_the_entity() {
    let generated = from_sql_with(SCHEMA, Opts::default());
    for (file, golden) in [
        ("stock_item.rs", "sql/expressions/stock_item.rs"),
        ("formula.rs", "sql/expressions/formula.rs"),
        ("stock_code.rs", "sql/expressions/stock_code.rs"),
    ] {
        generated.holds(file, golden);
    }

    let tables = parse_schema(SCHEMA).expect("schema should parse");
    let entities = [
        pgorm::Schema::new()
            .create_table_from_entity(stock_item::Entity)
            .expect("the entity's table is generated"),
        pgorm::Schema::new()
            .create_table_from_entity(formula::Entity)
            .expect("the entity's table is generated"),
        pgorm::Schema::new()
            .create_table_from_entity(stock_code::Entity)
            .expect("the entity's table is generated"),
    ];
    for (bridged, entity) in tables.iter().zip(&entities) {
        assert_eq!(expressions(entity), expressions(bridged));
    }
    let read = expressions(&tables[0]);
    assert_eq!(read.len(), 15, "{read:#?}");
    for expected in [
        "price DEFAULT SELECT 1.50",
        "note DEFAULT SELECT NULL",
        r#"total STORED SELECT "price" * "quantity""#,
        r#"spare VIRTUAL SELECT (("quantity" + 1) * 2) % 7"#,
    ] {
        assert!(
            read.iter().any(|found| found == expected),
            "{expected}: {read:#?}"
        );
    }
    assert_eq!(expressions(&tables[1]).len(), 26);
    assert_eq!(expressions(&tables[2]).len(), 1);
}

// [spec:pgorm:sem:codegen.entity.pk+5/test]    a key whose every column is generated is the
// database's to fill: the key carries no `auto_increment = false`
// [spec:pgorm:sem:codegen.entity.expressions+3/test]    and a VIRTUAL generated column in a key,
// which PostgreSQL does not support (0A000), is refused
#[test]
fn a_generated_key_is_the_databases() {
    assert!(<stock_code::PrimaryKey as pgorm::PrimaryKeyTrait>::auto_increment());
    for key in ["PRIMARY KEY", "UNIQUE"] {
        assert_eq!(
            refused(&format!(
                "CREATE TABLE t (a int, k int GENERATED ALWAYS AS (a) VIRTUAL {key});"
            )),
            "table `t` column `k`: a VIRTUAL generated column cannot be keyed, which PostgreSQL \
             does not support"
        );
    }
}

// [spec:pgorm:req:codegen.ddl.unsupported+21/test]    a construct outside the subset codegen
// reads is named with the expression it sits in, never approximated
#[test]
fn expressions_outside_the_subset_are_named() {
    let default = "the DEFAULT of column `t`.`b`";
    let generated = "the generation expression of column `t`.`b`";
    for (clause, construct, what) in [
        (
            "DEFAULT (SELECT 1)",
            "an expression codegen does not read",
            default,
        ),
        (
            "GENERATED ALWAYS AS (CASE a WHEN 0 THEN 1 END) STORED",
            "a CASE comparing an operand",
            generated,
        ),
        (
            "GENERATED ALWAYS AS (a BETWEEN 0 AND 1) STORED",
            "an operator codegen does not read",
            generated,
        ),
        (
            "DEFAULT 'x'::varchar(10)",
            "a cast to a modified type",
            default,
        ),
        (
            "DEFAULT app.next_code()",
            "a schema-qualified function",
            default,
        ),
        (
            "GENERATED ALWAYS AS (sum(a) OVER ()) STORED",
            "an aggregate, window or variadic call",
            generated,
        ),
        ("DEFAULT 1e10", "a literal", default),
        ("DEFAULT B'101'", "a literal", default),
        (
            "GENERATED ALWAYS AS (a IN (1, 2)) STORED",
            "an operator codegen does not read",
            generated,
        ),
        (
            "GENERATED ALWAYS AS (-a) STORED",
            "an operator codegen does not read",
            generated,
        ),
        (
            "DEFAULT CURRENT_USER",
            "a SQL value function other than CURRENT_DATE, CURRENT_TIME or CURRENT_TIMESTAMP",
            default,
        ),
        (
            "GENERATED ALWAYS AS (\"coalesce\"(a, 1)) STORED",
            "a function quoted to be named `coalesce`",
            generated,
        ),
        (
            "GENERATED ALWAYS AS (t.a + 1) STORED",
            "a qualified column reference",
            generated,
        ),
        (
            "DEFAULT substring('abc' from 2)",
            "a function written in SQL syntax",
            default,
        ),
    ] {
        assert_eq!(
            refused(&format!("CREATE TABLE t (a int, b int {clause});")),
            format!("unsupported DDL: {construct} in {what} at statement 1"),
            "{clause}"
        );
    }
}

// [spec:pgorm:sem:codegen.ddl.tables+13/test]    a column filled two ways, or a DEFAULT reading a
// column, is refused as PostgreSQL refuses it (42601, 0A000)
#[test]
fn a_column_filled_twice_is_refused() {
    for (column, first, second) in [
        ("a serial DEFAULT 1", "a serial type", "a DEFAULT"),
        (
            "a int GENERATED ALWAYS AS IDENTITY DEFAULT 1",
            "an identity",
            "a DEFAULT",
        ),
        (
            "a int DEFAULT 1 GENERATED ALWAYS AS (2) STORED",
            "a DEFAULT",
            "a generation expression",
        ),
        ("a int DEFAULT 1 DEFAULT 2", "a DEFAULT", "a DEFAULT"),
        (
            "a int GENERATED ALWAYS AS (1) GENERATED ALWAYS AS IDENTITY",
            "a generation expression",
            "an identity",
        ),
        (
            "a serial GENERATED ALWAYS AS (2) STORED",
            "a serial type",
            "a generation expression",
        ),
    ] {
        assert_eq!(
            refused(&format!("CREATE TABLE t ({column});")),
            format!(
                "statement 1: column `t`.`a` is filled by {first} and by {second}; a column takes one"
            ),
            "{column}"
        );
    }
    assert_eq!(
        refused("CREATE TABLE t (a int, b int DEFAULT a + 1);"),
        "statement 1: the DEFAULT of column `t`.`b` reads column `a`; a DEFAULT reads no column"
    );
}

// [spec:pgorm:sem:codegen.entity.transform+18/test]    a caller-built statement's expressions are
// read through the same subset, and what the entity could not hold is refused by name
// [spec:pgorm:sem:codegen.entity.expressions+3/test]    a caller-built searched CASE is read
// back arm by arm, a `Condition` arm as the expression it means
#[test]
fn caller_built_expressions_pass_the_gate() {
    let table = |column: ColumnDef| {
        keyed_with(
            "t",
            &["id"],
            vec![
                ColumnDef::new(n("id")).integer().not_null().to_owned(),
                column,
            ],
        )
    };
    for (column, problem) in [
        (
            ColumnDef::new(n("b"))
                .uuid()
                .default(Func::uuidv7())
                .to_owned(),
            "a DEFAULT codegen cannot read: a call of a function other than by name",
        ),
        (
            ColumnDef::new(n("b"))
                .integer()
                .default(
                    Func::named(n("pick"))
                        .arg(1)
                        .filter(Expr::col(n("id")).gt(0)),
                )
                .to_owned(),
            "a DEFAULT codegen cannot read: a call carrying DISTINCT or an aggregate clause",
        ),
        (
            ColumnDef::new(n("b"))
                .text()
                .generated(Expr::col(n("id")).like("x%"), GeneratedKind::Stored)
                .to_owned(),
            "a generation expression codegen cannot read: the operator Like",
        ),
        (
            ColumnDef::new(n("b"))
                .integer()
                .default(1)
                .generated(Expr::col(n("id")).add(1), GeneratedKind::Stored)
                .to_owned(),
            "a column takes one of a DEFAULT, a generation expression, an identity and the \
             serial family; PostgreSQL refuses two",
        ),
        (
            ColumnDef::new(n("b"))
                .integer()
                .default(Expr::col(n("id")))
                .to_owned(),
            "its DEFAULT reads column `id`; a DEFAULT reads no column",
        ),
        (
            ColumnDef::new(n("b"))
                .integer()
                .generated(Expr::col(n("gone")).add(1), GeneratedKind::Virtual)
                .to_owned(),
            "its generation expression reads column `gone`, which the table does not have",
        ),
    ] {
        assert_transform_error(
            vec![table(column)],
            &format!("table `t` column `b`: {problem}"),
        );
    }

    let read = generate(
        vec![table(
            ColumnDef::new(n("b"))
                .integer()
                .generated(Expr::col(n("id")).mul(2), GeneratedKind::Stored)
                .to_owned(),
        )],
        Opts::default(),
    );
    assert_contains(
        read.file("t.rs"),
        r#"#[pgorm(generated_stored = Expr::col(Column::Id).mul(Expr::val(2)))]
        pub b: Option<i32>,"#,
    );

    let cased = generate(
        vec![table(
            ColumnDef::new(n("b"))
                .integer()
                .generated(
                    Expr::case(Expr::col(n("id")).gt(0), Expr::col(n("id")))
                        .case(
                            pgorm_query::Condition::any()
                                .add(Expr::col(n("id")).eq(0))
                                .add(Expr::col(n("id")).lt(-9)),
                            0,
                        )
                        .finally(Expr::col(n("id")).mul(-1)),
                    GeneratedKind::Stored,
                )
                .to_owned(),
        )],
        Opts::default(),
    );
    assert_contains(
        cased.file("t.rs"),
        r#"#[pgorm(generated_stored = Expr::case(Expr::col(Column::Id).gt(Expr::val(0)), Expr::col(Column::Id))
            .case(Expr::expr(Expr::col(Column::Id).eq(Expr::val(0))).binary(pgorm::pgorm_query::BinOper::Or, Expr::col(Column::Id).lt(Expr::val(-9))), Expr::val(0))
            .finally(Expr::col(Column::Id).mul(Expr::val(-1))))]
        pub b: Option<i32>,"#,
    );
}
