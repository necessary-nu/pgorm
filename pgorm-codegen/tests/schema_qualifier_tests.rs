//! Schema qualifiers, from the DDL that spells them to the attribute the
//! generated entity carries — the identity that keeps a `tenant_a.item`
//! entity off `public.item` whatever `search_path` says.

mod common;

use common::*;
use pgorm_codegen::{Error, WriterOutput, entities_from_sql};

fn from_sql(sql: &str, opts: Opts) -> Generated {
    Generated {
        files: files(entities_from_sql(sql, opts).expect("schema should generate")),
    }
}

fn files(output: WriterOutput) -> Vec<(String, String)> {
    output
        .files
        .into_iter()
        .map(|file| (file.name, file.content))
        .collect()
}

#[track_caller]
fn assert_error(sql: &str, expected: &str) {
    match entities_from_sql(sql, Opts::default()) {
        Err(Error::TransformError(message)) => assert_eq!(message, expected),
        Err(other) => panic!("expected a TransformError, got {other:?}"),
        Ok(_) => panic!("expected an error, got generated entities"),
    }
}

const QUALIFIED: &str = "CREATE TABLE tenant_a.item (id int PRIMARY KEY);";

// [spec:pgorm:sem:codegen.entity.transform+16/test]    the source table's schema
// survives transformation and reaches the compact entity
// [spec:pgorm:def:codegen.entity.compact+2/test]
#[test]
fn compact_entity_carries_the_source_schema() {
    let generated = from_sql(QUALIFIED, Opts::default());

    assert_contains(
        generated.file("item.rs"),
        r#"#[pgorm(schema_name = "tenant_a", table_name = "item")]"#,
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    an unqualified table
// still generates no schema of its own
#[test]
fn an_unqualified_table_carries_no_schema() {
    let generated = from_sql("CREATE TABLE item (id int PRIMARY KEY);", Opts::default());

    assert_contains(
        generated.file("item.rs"),
        r#"#[pgorm(table_name = "item")]"#,
    );
    assert_not_contains(generated.file("item.rs"), "schema_name");
}

// [spec:pgorm:sem:codegen.entity.context+4/test]    the configured schema is a
// default, not an override: it fills in for tables the DDL left unqualified and
// never contradicts one that names its own schema
#[test]
fn the_configured_schema_is_only_a_default() {
    let generated = from_sql(
        "CREATE TABLE tenant_a.item (id int PRIMARY KEY);
         CREATE TABLE stock (id int PRIMARY KEY);",
        Opts {
            schema_name: Some("public".to_owned()),
            ..Default::default()
        },
    );

    assert_contains(
        generated.file("item.rs"),
        r#"#[pgorm(schema_name = "tenant_a", table_name = "item")]"#,
    );
    assert_contains(
        generated.file("stock.rs"),
        r#"#[pgorm(schema_name = "public", table_name = "stock")]"#,
    );
}

// [spec:pgorm:req:codegen.entity.collisions+2/test]    two schemas' same-named
// tables are two tables that want one file, and the gate says so
#[test]
fn same_name_in_two_schemas_is_refused() {
    assert_error(
        "CREATE TABLE tenant_a.item (id int PRIMARY KEY);
         CREATE TABLE tenant_b.item (id int PRIMARY KEY);",
        "tables `tenant_a.item` and `tenant_b.item` both generate the module name `item`: \
         same-named tables in different schemas are different tables, and need one generation run \
         each",
    );
}

// [spec:pgorm:req:codegen.entity.collisions+2/test]    a qualified table and an
// unqualified one of the same name collide the same way — whether they are one
// table is `search_path`'s business, and either way there is one `item.rs`
#[test]
fn qualified_and_unqualified_namesakes_are_refused() {
    assert_error(
        "CREATE TABLE tenant_a.item (id int PRIMARY KEY);
         CREATE TABLE item (id int PRIMARY KEY);",
        "tables `tenant_a.item` and `item` both generate the module name `item`: same-named \
         tables in different schemas are different tables, and need one generation run each",
    );
}

// [spec:pgorm:req:codegen.entity.collisions+2/test]    one identity declared
// twice is a duplicate, not a collision
#[test]
fn a_table_declared_twice_is_refused() {
    use pgorm_codegen::EntityTransformer;

    let twice = vec![qualified_item(), qualified_item()];

    match EntityTransformer::transform(twice) {
        Err(Error::TransformError(message)) => {
            assert_eq!(message, "table `tenant_a.item` is declared twice");
        }
        other => panic!("expected a TransformError, got {other:?}"),
    }
}

fn qualified_item() -> pgorm_query::TableCreateStatement {
    pgorm_query::Table::create((runtime_name("tenant_a"), runtime_name("item")))
        .col(serial("id"))
        .primary_key(runtime_name("id"))
        .to_owned()
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    a qualified foreign key
// resolves to the table it names, across schemas
#[test]
fn a_foreign_key_resolves_across_schemas() {
    let generated = from_sql(
        "CREATE TABLE tenant_a.item (id int PRIMARY KEY);
         CREATE TABLE tenant_b.cart (id int PRIMARY KEY, item_id int REFERENCES tenant_a.item(id));",
        Opts::default(),
    );

    assert_contains(
        generated.file("cart.rs"),
        r#"#[pgorm(schema_name = "tenant_b", table_name = "cart")]"#,
    );
    assert_contains(
        generated.file("cart.rs"),
        r#"belongs_to = "super::item::Entity""#,
    );
    assert_contains(
        generated.file("item.rs"),
        r#"has_many = "super::cart::Entity""#,
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    an unqualified reference
// resolves to the one table with that bare name, whatever schema it is in
#[test]
fn an_unqualified_key_reaches_a_qualified_table() {
    let generated = from_sql(
        "CREATE TABLE tenant_a.item (id int PRIMARY KEY);
         CREATE TABLE cart (id int PRIMARY KEY, item_id int REFERENCES item(id));",
        Opts::default(),
    );

    assert_contains(
        generated.file("cart.rs"),
        r#"belongs_to = "super::item::Entity""#,
    );
    assert_contains(
        generated.file("item.rs"),
        r#"has_many = "super::cart::Entity""#,
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    a key onto another
// schema's table is a key onto a table this schema does not define
#[test]
fn a_key_onto_another_schemas_table_is_unresolved() {
    assert_error(
        "CREATE TABLE tenant_a.item (id int PRIMARY KEY);
         CREATE TABLE cart (id int PRIMARY KEY, item_id int REFERENCES tenant_b.item(id));",
        "table `cart`: relation to `tenant_b.item` names a table the schema does not define",
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    self-reference is decided
// on identity: a qualified table keying itself is still `SelfRef`
#[test]
fn a_qualified_table_can_reference_itself() {
    let generated = from_sql(
        "CREATE TABLE tenant_a.node (id int PRIMARY KEY, parent int REFERENCES tenant_a.node(id));",
        Opts::default(),
    );

    assert_contains(generated.file("node.rs"), "SelfRef");
    assert_contains(
        generated.file("node.rs"),
        r#"belongs_to = "Entity", from = "Column::Parent", to = "Column::Id""#,
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    the gate names a table by
// its identity, so a failure in one of two schemas' tables says which
#[test]
fn a_refusal_names_the_qualified_table() {
    assert_error(
        "CREATE TABLE tenant_a.item (id int PRIMARY KEY, \"1st\" int);",
        "table `tenant_a.item` column `1st`: `1st` is not a valid Rust identifier",
    );
}

// [spec:pgorm:sem:codegen.ddl.objects+10/test]    an index attaches to the table
// its own name resolves to, not to whatever shares the bare name
#[test]
fn an_index_attaches_by_qualified_name() {
    assert_error(
        "CREATE TABLE tenant_a.item (id int PRIMARY KEY, sku text);
         CREATE UNIQUE INDEX item_sku ON tenant_b.item (sku);",
        "statement 2: no CREATE TABLE for table `tenant_b.item`",
    );
}

// [spec:pgorm:sem:codegen.ddl.objects+10/test]    and a qualified index that does
// name its table is folded into it, giving the column its `unique`
#[test]
fn a_qualified_index_reaches_its_own_table() {
    let generated = from_sql(
        "CREATE TABLE tenant_a.item (id int PRIMARY KEY, sku text);
         CREATE UNIQUE INDEX item_sku ON tenant_a.item (sku);",
        Opts::default(),
    );

    assert_contains(
        generated.file("item.rs"),
        r#"#[pgorm(column_type = ColumnType::Text, nullable, unique)] pub sku"#,
    );
}

// [spec:pgorm:sem:codegen.ddl.objects+10/test]    a comment resolves the same way
#[test]
fn a_comment_attaches_by_qualified_name() {
    assert_error(
        "CREATE TABLE tenant_a.item (id int PRIMARY KEY);
         COMMENT ON TABLE tenant_b.item IS 'the other one';",
        "statement 2: no CREATE TABLE for table `tenant_b.item`",
    );
}

// [spec:pgorm:sem:codegen.ddl.objects+10/test]    an unqualified reference that
// two tables answer to is named as such, rather than attached to one of them
#[test]
fn an_ambiguous_unqualified_index_is_refused() {
    assert_error(
        "CREATE TABLE tenant_a.item (id int PRIMARY KEY, sku text);
         CREATE TABLE tenant_b.item (id int PRIMARY KEY, sku text);
         CREATE UNIQUE INDEX item_sku ON item (sku);",
        "statement 3: `item` names more than one table",
    );
}

// [spec:pgorm:sem:codegen.ddl.tables+11/test]    the bridge preserved the
// qualifier all along; it is the whole pipeline that now keeps it
#[test]
fn bridge_and_generated_entity_agree_on_schema() {
    use pgorm_codegen::parse_schema;
    use pgorm_query::TableName;

    let statements = parse_schema(QUALIFIED).expect("schema should parse");
    let [statement] = statements.as_slice() else {
        panic!("expected one statement, got {}", statements.len());
    };

    assert!(matches!(
        statement.get_table_name(),
        TableName::SchemaTable(schema, table)
            if schema.to_string() == "tenant_a" && table.to_string() == "item"
    ));
    assert_contains(
        from_sql(QUALIFIED, Opts::default()).file("item.rs"),
        r#"schema_name = "tenant_a""#,
    );
}

// [spec:pgorm:sem:codegen.entity.enums+3/test]    a schema-qualified enum's
// schema survives into the generated DeriveActiveEnum attribute, so the
// entity round-trips the type it was described with
#[test]
fn qualified_enum_preserves_its_schema() {
    let generated = from_sql(
        "CREATE TYPE custom.status AS ENUM ('open'); \
         CREATE TABLE custom.task (id integer PRIMARY KEY, status custom.status NOT NULL);",
        Opts::default(),
    );

    assert_contains(
        generated.file("pgorm_active_enums.rs"),
        r#"schema_name = "custom""#,
    );
}

// [spec:pgorm:sem:codegen.ddl.types+9/test]    a qualified reference resolves
// only its exact identity: a same-named enum under another qualification MUST
// NOT satisfy it
#[test]
fn qualified_enum_does_not_resolve_another_schema() {
    assert_error(
        "CREATE TYPE a.status AS ENUM ('open'); \
         CREATE TABLE task (id integer PRIMARY KEY, status b.status NOT NULL);",
        "statement 2: type `b.status` on column `task`.`status` is not declared; \
         a same-named enum under a different qualification does not resolve it",
    );
}

// [spec:pgorm:sem:codegen.ddl.types+9/test]    nor does an unqualified
// reference resolve a type declared only under a schema
#[test]
fn unqualified_reference_needs_an_unqualified_declaration() {
    assert_error(
        "CREATE TYPE a.status AS ENUM ('open'); \
         CREATE TABLE task (id integer PRIMARY KEY, status status NOT NULL);",
        "statement 2: type `status` on column `task`.`status` is not declared; \
         a same-named enum under a different qualification does not resolve it",
    );
}

// [spec:pgorm:sem:codegen.entity.enums+3/test]    one bare Rust enum cannot
// stand for two differently-qualified database types: the collision is
// refused rather than generated
#[test]
fn same_named_enums_in_two_schemas_are_refused() {
    assert_error(
        "CREATE TYPE a.status AS ENUM ('open'); \
         CREATE TYPE b.status AS ENUM ('closed'); \
         CREATE TABLE t1 (id integer PRIMARY KEY, s a.status NOT NULL); \
         CREATE TABLE t2 (id integer PRIMARY KEY, s b.status NOT NULL);",
        "enum `status` is used under two qualifications; the generated Rust \
         enum can carry only one",
    );
}
