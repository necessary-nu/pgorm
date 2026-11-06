//! What the macro leaves behind: the literal it was given, and nothing else.

use pgorm_sql_macro::sql;

// [spec:pgorm:def:macros.sql+6/test]    the expansion is the literal, in const position
#[test]
fn expansion_is_the_literal_unchanged() {
    const QUERY: &str = sql!("SELECT id, name FROM cake WHERE id = $1");

    assert_eq!(QUERY, "SELECT id, name FROM cake WHERE id = $1");
    assert_eq!(
        sql!(r#"SELECT "cake"."id" FROM "cake""#),
        r#"SELECT "cake"."id" FROM "cake""#
    );
}

// [spec:pgorm:def:macros.sql+6/test]    a `&'static str`, so it outlives any frame
#[test]
fn expansion_is_a_static_str() {
    fn keep(query: &'static str) -> &'static str {
        query
    }

    assert_eq!(keep(sql!("SELECT 1")), "SELECT 1");
}

// [spec:pgorm:sem:macros.sql.script/test]    every statement of a script is checked
#[test]
fn multi_statement_literal_passes_through() {
    const SCRIPT: &str = sql!(
        "CREATE TABLE cake (id int primary key);
         INSERT INTO cake (id) VALUES (1), (2);
         SELECT count(*) FROM cake;"
    );

    assert!(SCRIPT.starts_with("CREATE TABLE cake"));
    assert_eq!(SCRIPT.matches(';').count(), 3);
}

// [spec:pgorm:sem:macros.sql.script/test]    a script with no statements in it
#[test]
fn empty_and_comment_only_literals_pass() {
    assert_eq!(sql!(""), "");
    assert_eq!(sql!("-- nothing to run"), "-- nothing to run");
    assert_eq!(sql!(";"), ";");
}

// [spec:pgorm:req:macros.sql.ceiling/test]    no catalog: unknown names are fine
#[test]
fn unknown_tables_and_columns_pass() {
    assert_eq!(
        sql!("SELECT nowhere FROM no_such_table JOIN nor_this USING (nope)"),
        "SELECT nowhere FROM no_such_table JOIN nor_this USING (nope)"
    );
}

// [spec:pgorm:req:macros.sql.ceiling/test]    no catalog: type modifiers unchecked
#[test]
fn misapplied_type_modifiers_pass() {
    assert_eq!(
        sql!("CREATE TABLE t (amount money(12, 2))"),
        "CREATE TABLE t (amount money(12, 2))"
    );
}

// [spec:pgorm:def:macros.sql+6/test]    the grammar is PostgreSQL 18's: statements
// only 18 parses compile, where PostgreSQL 17's grammar refused every one
#[test]
fn postgres_18_grammar_is_accepted() {
    assert_eq!(
        sql!("CREATE TABLE t (a int, b int GENERATED ALWAYS AS (a * 2) VIRTUAL)"),
        "CREATE TABLE t (a int, b int GENERATED ALWAYS AS (a * 2) VIRTUAL)"
    );
    assert_eq!(
        sql!(
            "CREATE TABLE t (id int, during tstzrange, PRIMARY KEY (id, during WITHOUT OVERLAPS))"
        ),
        "CREATE TABLE t (id int, during tstzrange, PRIMARY KEY (id, during WITHOUT OVERLAPS))"
    );
    assert_eq!(
        sql!("CREATE TABLE t (id int, CONSTRAINT id_present NOT NULL id NO INHERIT)"),
        "CREATE TABLE t (id int, CONSTRAINT id_present NOT NULL id NO INHERIT)"
    );
    assert_eq!(
        sql!("UPDATE t SET a = a + 1 RETURNING WITH (OLD AS o, NEW AS n) o.a, n.a"),
        "UPDATE t SET a = a + 1 RETURNING WITH (OLD AS o, NEW AS n) o.a, n.a"
    );
}
