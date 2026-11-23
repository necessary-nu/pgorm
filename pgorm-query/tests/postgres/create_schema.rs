use super::*;
use crate::oracle::{assert_eq, parsed_nodes};
use pgsql_syntax::protobuf::{DropBehavior, ObjectType};

fn n(name: &str) -> Name {
    Name::runtime(name)
}

/// The one `kind` node in `sql`, as PostgreSQL's parser read it.
fn statement(sql: &str, kind: &str) -> serde_json::Value {
    let mut found = parsed_nodes(sql, kind);
    assert_eq!(found.len(), 1, "exactly one {kind} in {sql}");
    found.remove(0)
}

/// The schemas a `DropStmt` names, each one bare `String` node.
fn dropped(drop: &serde_json::Value) -> Vec<&str> {
    drop["objects"]
        .as_array()
        .expect("the dropped names")
        .iter()
        .map(|object| object["String"]["sval"].as_str().expect("one name"))
        .collect()
}

// [spec:pgorm:req:sql.ddl.schema/test]    a create names its schema and nothing else, and
// `IF NOT EXISTS` is read back as the parser's flag for it
#[test]
fn a_schema_needs_only_its_name() {
    let sql = Schema::create(n("billing")).to_string();
    assert_eq!(sql, r#"CREATE SCHEMA "billing""#);
    let create = statement(&sql, "CreateSchemaStmt");
    assert_eq!(create["schemaname"], "billing");
    assert_eq!(create["if_not_exists"], false);
    assert!(create.get("authrole").is_none(), "{sql}");
    assert!(
        create["schema_elts"]
            .as_array()
            .is_none_or(|elements| elements.is_empty()),
        "{sql}"
    );

    let sql = Schema::create(n("billing")).if_not_exists().to_string();
    assert_eq!(sql, r#"CREATE SCHEMA IF NOT EXISTS "billing""#);
    assert_eq!(statement(&sql, "CreateSchemaStmt")["if_not_exists"], true);
}

// [spec:pgorm:req:sql.ddl.schema/test]    a schema name is one quoted identifier, its own
// quote doubled and its case kept, wherever a statement writes it
#[test]
fn schema_names_are_quoted_identifiers() {
    let odd = r#"Bill "ing".v2"#;
    let sql = Schema::create(n(odd)).to_string();
    assert_eq!(sql, r#"CREATE SCHEMA "Bill ""ing"".v2""#);
    assert_eq!(statement(&sql, "CreateSchemaStmt")["schemaname"], odd);

    let sql = Schema::drop(n(odd)).to_string();
    assert_eq!(sql, r#"DROP SCHEMA "Bill ""ing"".v2""#);
    assert_eq!(dropped(&statement(&sql, "DropStmt")), [odd]);

    let sql = Schema::rename(n(odd), n("Ledger")).to_string();
    assert_eq!(sql, r#"ALTER SCHEMA "Bill ""ing"".v2" RENAME TO "Ledger""#);
    let rename = statement(&sql, "RenameStmt");
    assert_eq!(rename["subname"], odd);
    assert_eq!(rename["newname"], "Ledger");
}

// [spec:pgorm:req:sql.ddl.schema/test]    a drop names one or more schemas in order, with
// `IF EXISTS` and a behavior, the last of cascade and restrict winning
#[test]
fn a_drop_names_schemas_in_order() {
    let sql = Schema::drop(n("a")).to_string();
    assert_eq!(sql, r#"DROP SCHEMA "a""#);
    let drop = statement(&sql, "DropStmt");
    assert_eq!(drop["remove_type"], ObjectType::ObjectSchema as i32);
    assert_eq!(drop["behavior"], DropBehavior::DropRestrict as i32);
    assert_eq!(drop["missing_ok"], false);
    assert_eq!(dropped(&drop), ["a"]);

    let statement_builder = Schema::drop(n("a"))
        .name(n("b"))
        .name(n("c"))
        .if_exists()
        .cascade()
        .to_owned();
    assert_eq!(
        statement_builder
            .names()
            .map(|name| name.to_string())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    let sql = statement_builder.to_string();
    assert_eq!(sql, r#"DROP SCHEMA IF EXISTS "a", "b", "c" CASCADE"#);
    let drop = statement(&sql, "DropStmt");
    assert_eq!(drop["remove_type"], ObjectType::ObjectSchema as i32);
    assert_eq!(drop["behavior"], DropBehavior::DropCascade as i32);
    assert_eq!(drop["missing_ok"], true);
    assert_eq!(dropped(&drop), ["a", "b", "c"]);

    let sql = Schema::drop(n("a")).cascade().restrict().to_string();
    assert_eq!(sql, r#"DROP SCHEMA "a" RESTRICT"#);
    assert_eq!(
        statement(&sql, "DropStmt")["behavior"],
        DropBehavior::DropRestrict as i32
    );
    let sql = Schema::drop(n("a")).restrict().cascade().to_string();
    assert_eq!(sql, r#"DROP SCHEMA "a" CASCADE"#);
}

// [spec:pgorm:req:sql.ddl.schema/test]    a rename is `ALTER SCHEMA .. RENAME TO`, read back as
// a schema rename from the one name to the other
#[test]
fn a_rename_moves_the_schema_name() {
    let sql = Schema::rename(n("billing"), n("invoicing")).to_string();
    assert_eq!(sql, r#"ALTER SCHEMA "billing" RENAME TO "invoicing""#);
    let rename = statement(&sql, "RenameStmt");
    assert_eq!(rename["rename_type"], ObjectType::ObjectSchema as i32);
    assert_eq!(rename["subname"], "billing");
    assert_eq!(rename["newname"], "invoicing");
    assert_eq!(rename["missing_ok"], false);
}

// [spec:pgorm:req:sql.ddl.schema/test]    the grammar has no `IF EXISTS` on a schema rename,
// which is why the statement offers none, and no qualified schema name
#[test]
fn the_grammar_refuses_what_is_not_built() {
    crate::oracle::assert_rejected(r#"ALTER SCHEMA IF EXISTS "a" RENAME TO "b""#);
    crate::oracle::assert_rejected(r#"CREATE SCHEMA "db"."a""#);
    crate::oracle::assert_rejected(r#"DROP SCHEMA "#);
}
