//! What the macro leaves behind: the literal it was given, typed by what its
//! statement may write.

use pgorm::{Reads, Sql, Writes};
use pgorm_sql_macro::sql;

// [spec:pgorm:def:macros.sql+10/test]    the expansion is the literal, in const position
#[test]
fn expansion_is_the_literal_unchanged() {
    const QUERY: &Sql<Reads> = sql!("SELECT id, name FROM cake WHERE id = $1");

    assert_eq!(QUERY, "SELECT id, name FROM cake WHERE id = $1");
    assert_eq!(
        sql!(r#"SELECT "cake"."id" FROM "cake""#),
        r#"SELECT "cake"."id" FROM "cake""#
    );
}

// [spec:pgorm:req:sql.target+4/test]    the grammar is PostgreSQL 19's, so
// what 19 added compiles
#[test]
fn nineteen_only_syntax_compiles() {
    const DO_SELECT: &str =
        sql!("INSERT INTO cake (id) VALUES ($1) ON CONFLICT (id) DO SELECT RETURNING *").as_str();
    const REPACK: &str = sql!("REPACK cake USING INDEX cake_pkey").as_str();

    assert!(DO_SELECT.ends_with("DO SELECT RETURNING *"));
    assert_eq!(REPACK, "REPACK cake USING INDEX cake_pkey");
}

// [spec:pgorm:def:macros.sql+10/test]    a `&'static Sql`, so it outlives any frame, and dereferences to `str`
#[test]
fn expansion_is_static() {
    fn keep(query: &'static str) -> &'static str {
        query
    }

    assert_eq!(keep(sql!("SELECT 1")), "SELECT 1");
}

// [spec:pgorm:def:read-only.sql-macro/test]    a statement that only reads is `Sql<Reads>`, one that may write `Sql<Writes>`, by its whole tree
#[test]
fn expansion_is_typed_by_what_it_writes() {
    let reads: [&Sql<Reads>; 6] = [
        sql!("SELECT 1"),
        sql!("VALUES (1)"),
        sql!("TABLE cake"),
        sql!("SHOW search_path"),
        sql!("EXPLAIN DELETE FROM cake"),
        sql!("WITH last AS (SELECT max(id) FROM cake) SELECT * FROM last"),
    ];
    let writes: [&Sql<Writes>; 6] = [
        sql!("DELETE FROM cake"),
        sql!("WITH gone AS (DELETE FROM cake RETURNING id) SELECT * FROM gone"),
        sql!("SELECT * FROM cake FOR UPDATE"),
        sql!("SELECT * INTO cake_copy FROM cake"),
        sql!("EXPLAIN ANALYZE SELECT * FROM cake"),
        sql!("SELECT 1; DELETE FROM cake"),
    ];
    assert!(reads.iter().all(|sql| !sql.is_empty()));
    assert!(writes.iter().all(|sql| !sql.is_empty()));
}

// [spec:pgorm:sem:macros.sql.script+2/test]    every statement of a script is checked
#[test]
fn multi_statement_literal_passes_through() {
    const SCRIPT: &str = sql!(
        "CREATE TABLE cake (id int primary key);
         INSERT INTO cake (id) VALUES (1), (2);
         SELECT count(*) FROM cake;"
    )
    .as_str();

    assert!(SCRIPT.starts_with("CREATE TABLE cake"));
    assert_eq!(SCRIPT.matches(';').count(), 3);
}

// [spec:pgorm:sem:macros.sql.script+2/test]    a script with no statements in it
#[test]
fn empty_and_comment_only_literals_pass() {
    assert_eq!(sql!(""), "");
    assert_eq!(sql!("-- nothing to run"), "-- nothing to run");
    assert_eq!(sql!(";"), ";");
}

// [spec:pgorm:req:macros.sql.ceiling+2/test]    no catalog: unknown names are fine
#[test]
fn unknown_tables_and_columns_pass() {
    assert_eq!(
        sql!("SELECT nowhere FROM no_such_table JOIN nor_this USING (nope)"),
        "SELECT nowhere FROM no_such_table JOIN nor_this USING (nope)"
    );
}

// [spec:pgorm:req:macros.sql.ceiling+2/test]    no catalog: type modifiers unchecked
#[test]
fn misapplied_type_modifiers_pass() {
    assert_eq!(
        sql!("CREATE TABLE t (amount money(12, 2))"),
        "CREATE TABLE t (amount money(12, 2))"
    );
}

// [spec:pgorm:def:macros.sql+10/test]    what PostgreSQL 18's grammar added
// compiles under 19's, where PostgreSQL 17's grammar refused every one
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
