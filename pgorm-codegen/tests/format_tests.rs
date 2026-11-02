//! The output format, the `Model` attribute assembly, and the import block.

mod common;

use common::*;
use pgorm_codegen::WithSerde;
use pgorm_query::{ColumnDef, ColumnType, Name, StringLen, Table};

// [spec:pgorm:req:macros.derive.one-form/test]    codegen writes the one form
// [spec:pgorm:def:codegen.entity.compact+4/test]    the compact format emits
// imports, Model and Relation, in order, and leaves the `Related` impl one
// relation reaches and the behaviour with no hooks to the derives
#[test]
fn compact_format_emits_its_blocks_in_order() {
    let generated = generate(vec![cake(), fruit()], Opts::default());
    let fruit = generated.file("fruit.rs");
    let blocks = blocks(fruit);

    assert_eq!(blocks.len(), 3, "{blocks:#?}");
    assert_starts_with(blocks[0], "use pgorm::entity::prelude::*;");
    assert_starts_with(
        blocks[1],
        r#"#[derive(Clone, Debug, PartialEq, Entity, Eq)]
           #[pgorm(table_name = "fruit")]
           pub struct Model"#,
    );
    assert_starts_with(
        blocks[2],
        "#[derive(Copy, Clone, Debug, Relation)] pub enum Relation",
    );
    assert_not_contains(fruit, "impl Related");
    assert_not_contains(fruit, "ActiveModelBehavior");
}

// [spec:pgorm:def:codegen.entity.compact+4/test]    the `schema_name` part of the
// Model attribute appears only when configured, and an entity with no relations
// emits the empty `pub enum Relation {}`
#[test]
fn compact_model_attribute_and_empty_relation_enum() {
    let plain = generate(vec![cake()], Opts::default());
    assert_contains(plain.file("cake.rs"), r#"#[pgorm(table_name = "cake")]"#);
    assert_contains(plain.file("cake.rs"), "pub enum Relation { }");

    let scoped = generate(
        vec![cake()],
        Opts {
            schema_name: Some("public".to_owned()),
            ..Default::default()
        },
    );
    assert_contains(
        scoped.file("cake.rs"),
        r#"#[pgorm(schema_name = "public", table_name = "cake")]"#,
    );
}

// [spec:pgorm:sem:codegen.entity.compact.attrs+12/test]    the `#[pgorm(..)]` field
// attribute assembles its parts in one fixed order
#[test]
fn compact_field_attribute_parts_assembled_in_fixed_order() {
    let generated = generate(
        vec![
            Table::create(Name::runtime("ledger"))
                .col(
                    ColumnDef::new_with_type(
                        Name::runtime("camelCase"),
                        ColumnType::Decimal(Some((10, 2))),
                    )
                    .to_owned(),
                )
                .primary_key(Name::runtime("camelCase"))
                .unique(unique_key("ledger", "camelCase"))
                .to_owned(),
        ],
        Opts::default(),
    );

    assert_contains(
        generated.file("ledger.rs"),
        r#"#[pgorm(
            column_name = "camelCase",
            primary_key,
            auto_increment = false,
            column_type = ColumnType::Decimal(Some((10u32, 2u32))),
            nullable,
            unique
        )]
        pub camel_case: Option<Decimal>,"#,
    );
}

// [spec:pgorm:req:macros.derive.one-form/test]    codegen states a `column_type` as the
// bare expression the derive takes
// [spec:pgorm:sem:codegen.entity.compact.attrs+12/test]    `column_type` is emitted
// on every type its field's Rust type does not infer — a length, `char` of any length,
// a precision and scale, `money`, `text`, `jsonb`, a named type — and on `Float`,
// `Double` and `Bytea`, spelled as the type's whole `ColumnType` expression
#[test]
fn compact_column_type_attribute_covers_ambiguous_types() {
    let generated = generate(
        vec![keyed_with(
            "sample",
            &["id"],
            vec![
                serial("id"),
                typed("a_float", ColumnType::Float),
                typed("a_double", ColumnType::Double),
                typed("a_decimal", ColumnType::Decimal(Some((10, 2)))),
                typed("a_money", ColumnType::Money),
                typed("a_text", ColumnType::Text),
                typed("a_jsonb", ColumnType::JsonBinary),
                typed("a_named", ColumnType::named("citext")),
                typed("a_bytea", ColumnType::Bytea),
                typed("a_varchar", ColumnType::String(StringLen::N(3))),
                typed("a_longest", ColumnType::String(StringLen::Max)),
                typed("a_char", ColumnType::Char(Some(2))),
                typed("a_bpchar", ColumnType::Char(None)),
            ],
        )],
        Opts::default(),
    );
    let sample = generated.file("sample.rs");

    for (field, rendered) in [
        ("a_float", "Float"),
        ("a_double", "Double"),
        ("a_decimal", "Decimal(Some((10u32, 2u32)))"),
        ("a_money", "Money"),
        ("a_text", "Text"),
        ("a_jsonb", "JsonBinary"),
        ("a_named", r#"named("citext")"#),
        ("a_bytea", "Bytea"),
        ("a_varchar", "String(StringLen::N(3u32))"),
        ("a_longest", "String(StringLen::Max)"),
        ("a_char", "Char(Some(2u32))"),
        ("a_bpchar", "Char(None)"),
    ] {
        assert_contains(
            sample,
            &format!(r#"#[pgorm(column_type = ColumnType::{rendered})] pub {field}:"#),
        );
    }
}

// [spec:pgorm:sem:codegen.entity.compact.attrs+12/test]    a field needing none of
// the parts carries no `#[pgorm]` attribute, and `nullable` never appears
// without a `column_type`
#[test]
fn compact_fields_without_applicable_parts_carry_no_attribute() {
    let generated = generate(
        vec![keyed_with(
            "task",
            &["id"],
            vec![
                serial("id"),
                typed("count", ColumnType::Integer),
                typed_null("maybe", ColumnType::Integer),
                typed_null("note", ColumnType::Text),
            ],
        )],
        Opts::default(),
    );
    let task = generated.file("task.rs");

    // nothing sits between the fields, so neither carries an attribute
    assert_contains(
        task,
        "#[pgorm(primary_key)] pub id: i32, pub count: i32, pub maybe: Option<i32>,",
    );
    // `nullable` rides along only with a `column_type`
    assert_contains(
        task,
        r#"#[pgorm(column_type = ColumnType::Text, nullable)] pub note: Option<String>,"#,
    );
}

// [spec:pgorm:sem:codegen.entity.compact.model+2/test]    derives, struct
// attribute, extra attributes and fields are emitted as one ordered block
#[test]
fn compact_model_assembles_derives_attributes_fields_in_order() {
    let generated = generate(
        vec![
            Table::create(Name::runtime("cake"))
                .col(serial("id"))
                .primary_key(Name::runtime("id"))
                // not snake_case: the raw DB name is preserved by `column_name`
                .col(
                    ColumnDef::new_with_type(Name::runtime("bakedAt"), ColumnType::Timestamp)
                        .not_null()
                        .to_owned(),
                )
                // a Rust keyword: escaped as a raw identifier
                .col(typed("type", ColumnType::Integer))
                .to_owned(),
        ],
        Opts {
            with_serde: WithSerde::Both,
            schema_name: Some("public".to_owned()),
            model_extra_derives: vec!["Default".to_owned()],
            model_extra_attributes: vec![r#"serde(rename_all = "camelCase")"#.to_owned()],
            ..Default::default()
        },
    );

    assert_contains(
        generated.file("cake.rs"),
        r#"#[derive(Clone, Debug, PartialEq, Entity, Eq, Serialize, Deserialize, Default)]
           #[pgorm(schema_name = "public", table_name = "cake")]
           #[serde(rename_all = "camelCase")]
           pub struct Model {
               #[pgorm(primary_key)]
               pub id: i32,
               #[pgorm(column_name = "bakedAt")]
               pub baked_at: DateTime,
               pub r#type: i32,
           }"#,
    );
}

// [spec:pgorm:sem:codegen.entity.compact.model+2/test]    primary-key membership
// is decided against the raw DB column name, not the snake_case field name
#[test]
fn compact_model_pk_membership_uses_raw_column_name() {
    let generated = generate(
        vec![keyed_with(
            "ledger",
            &["entryId"],
            vec![
                ColumnDef::new_with_type(Name::runtime("entryId"), ColumnType::Integer)
                    .not_null()
                    .to_owned(),
            ],
        )],
        Opts::default(),
    );

    assert_contains(
        generated.file("ledger.rs"),
        r#"#[pgorm(column_name = "entryId", primary_key, auto_increment = false)] pub entry_id: i32,"#,
    );
}

// [spec:pgorm:def:codegen.entity.compact+4/test]    each section is one
// contiguous blank-line-separated block, with the generated header prepended
#[test]
fn sections_are_one_contiguous_block_each() {
    let generated = generate(vec![cake(), fruit()], Opts::default());
    let content = generated.file("fruit.rs");

    assert!(content.starts_with("//! `pgorm` Entity, @generated by pgorm-codegen"));
    for block in blocks(content) {
        assert!(
            !block.contains('\n'),
            "each section should be a single unformatted block: {block}"
        );
        // every block lexes on its own, i.e. sections do not straddle the join
        let _ = norm(block);
    }
}

// [spec:pgorm:sem:codegen.entity.imports+3/test]    the import block is the prelude
// import plus the serde import selected by `WithSerde`
#[test]
fn import_block_matches_the_with_serde_variant() {
    for (with_serde, expected) in [
        (WithSerde::None, "use pgorm::entity::prelude::*;"),
        (
            WithSerde::Serialize,
            "use pgorm::entity::prelude::*; use serde::Serialize;",
        ),
        (
            WithSerde::Deserialize,
            "use pgorm::entity::prelude::*; use serde::Deserialize;",
        ),
        (
            WithSerde::Both,
            "use pgorm::entity::prelude::*; use serde::{Deserialize, Serialize};",
        ),
    ] {
        let generated = generate(
            vec![cake()],
            Opts {
                with_serde,
                ..Default::default()
            },
        );
        let blocks = blocks(generated.file("cake.rs"));
        assert_eq!(norm(blocks[0]), norm(expected));
    }
}

// [spec:pgorm:sem:codegen.entity.imports+3/test]    entity files import each active
// enum once, in first-use column order, looking through `Array`
#[test]
fn entity_imports_each_enum_once_in_first_use() {
    let alpha = || ColumnType::Enum {
        schema: None,
        name: Name::runtime("alpha"),
        variants: vec![Name::runtime("one")],
    };

    let generated = generate(
        vec![keyed_with(
            "task",
            &["id"],
            vec![
                serial("id"),
                // `zeta` is used first even though `alpha` sorts before it
                enum_col("first", "zeta", &["a", "b"]),
                enum_col("second", "alpha", &["one"]),
                enum_col("third", "zeta", &["a", "b"]),
                ColumnDef::new(Name::runtime("fourth"))
                    .array(alpha())
                    .not_null()
                    .to_owned(),
            ],
        )],
        Opts::default(),
    );
    let blocks = blocks(generated.file("task.rs"));

    assert_eq!(
        norm(blocks[0]),
        norm(
            "use pgorm::entity::prelude::*;
             use super::pgorm_active_enums::Zeta;
             use super::pgorm_active_enums::Alpha;"
        )
    );
}

// [spec:pgorm:sem:codegen.entity.imports+3/test]    `pgorm_active_enums.rs` heads
// with the same import block, without any enum imports, directly below the
// generated-file header
#[test]
fn active_enums_file_imports_serde_not_enum_modules() {
    let generated = generate(
        vec![keyed_with(
            "task",
            &["id"],
            vec![serial("id"), enum_col("state", "task_state", &["open"])],
        )],
        Opts {
            with_serde: WithSerde::Both,
            ..Default::default()
        },
    );

    let enums = generated.file("pgorm_active_enums.rs");
    let lines: Vec<&str> = enums.lines().filter(|l| !l.is_empty()).collect();
    assert!(lines[0].starts_with("//! `pgorm` Entity, @generated by pgorm-codegen"));
    assert_eq!(
        norm(lines[1]),
        norm("use pgorm::entity::prelude::*; use serde::{Deserialize, Serialize};")
    );
    assert_not_contains(enums, "use super::pgorm_active_enums::TaskState;");
}

// [spec:pgorm:sem:codegen.entity.imports+3/test]    in entity files the imports sit
// directly below the generated-file header
#[test]
fn imports_sit_directly_below_the_generated_header() {
    let generated = generate(vec![cake()], Opts::default());
    let content = generated.file("cake.rs");

    assert!(content.starts_with("//! `pgorm` Entity, @generated by pgorm-codegen"));
    assert_starts_with(blocks(content)[0], "use pgorm::entity::prelude::*;");
}
