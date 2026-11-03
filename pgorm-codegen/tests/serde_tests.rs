//! `WithSerde`: parsing, the import + derive fragment it contributes, and the
//! two field-level skip flags.

mod common;

use common::*;
use pgorm_codegen::{Error, WithSerde};
use pgorm_query::{ColumnDef, ColumnType, Name};
use std::str::FromStr;

fn task_schema() -> Vec<pgorm_query::TableCreateStatement> {
    vec![keyed_with(
        "task",
        &["id"],
        vec![
            serial("id"),
            typed("_secret", ColumnType::Integer),
            enum_col("state", "task_state", &["open", "done"]),
        ],
    )]
}

// [spec:pgorm:def:codegen.entity.serde+1/test]    the four variant names parse; any
// other string is a `TransformError`
#[test]
fn with_serde_parses_its_four_variant_names() {
    assert_eq!(WithSerde::from_str("none").unwrap(), WithSerde::None);
    assert_eq!(
        WithSerde::from_str("serialize").unwrap(),
        WithSerde::Serialize
    );
    assert_eq!(
        WithSerde::from_str("deserialize").unwrap(),
        WithSerde::Deserialize
    );
    assert_eq!(WithSerde::from_str("both").unwrap(), WithSerde::Both);

    match WithSerde::from_str("Both") {
        Err(Error::TransformError(msg)) => {
            assert_eq!(msg, "Unsupported enum variant 'Both'");
        }
        other => panic!("expected a TransformError, got {other:?}"),
    }
}

// [spec:pgorm:def:codegen.entity.serde+1/test]    each variant contributes its
// import and its derives, to Models and to generated active enums alike
#[test]
fn with_serde_contributes_matching_imports_and_derives() {
    for (variant, import, derives) in [
        (WithSerde::None, None, ""),
        (
            WithSerde::Serialize,
            Some("use serde::Serialize;"),
            ", Serialize",
        ),
        (
            WithSerde::Deserialize,
            Some("use serde::Deserialize;"),
            ", Deserialize",
        ),
        (
            WithSerde::Both,
            Some("use serde::{Deserialize, Serialize};"),
            ", Serialize, Deserialize",
        ),
    ] {
        let generated = generate(
            task_schema(),
            Opts {
                with_serde: variant,
                ..Default::default()
            },
        );
        let task = generated.file("task.rs");
        let enums = generated.file("pgorm_active_enums.rs");

        match import {
            Some(import) => {
                assert_contains(task, import);
                assert_contains(enums, import);
            }
            None => {
                assert_not_contains(task, "use serde::Serialize;");
                assert_not_contains(task, "use serde::Deserialize;");
                assert_not_contains(enums, "use serde::Serialize;");
                assert_not_contains(enums, "use serde::Deserialize;");
            }
        }

        assert_contains(
            task,
            &format!("#[derive(Clone, Debug, PartialEq, Entity, Eq{derives})]"),
        );
        assert_contains(
            enums,
            &format!("#[derive(Debug, Clone, PartialEq, Eq, ActiveEnum{derives})]"),
        );
    }
}

// [spec:pgorm:sem:codegen.entity.serde.skip/test]    `serde_skip_hidden_column`
// puts `#[serde(skip)]` on every `_`-prefixed column, under any serde variant
#[test]
fn serde_skip_hidden_column_marks_underscore_prefixed_columns() {
    for variant in [
        WithSerde::Serialize,
        WithSerde::Deserialize,
        WithSerde::Both,
    ] {
        let generated = generate(
            task_schema(),
            Opts {
                with_serde: variant,
                serde_skip_hidden_column: true,
                ..Default::default()
            },
        );
        assert_contains(generated.file("task.rs"), "#[serde(skip)] pub secret: i32,");
        // ordinary columns are untouched
        assert_contains(generated.file("task.rs"), "pub state: TaskState,");
        assert_not_contains(generated.file("task.rs"), "#[serde(skip)] pub state");
    }
}

// [spec:pgorm:sem:codegen.entity.serde.skip/test]
// `serde_skip_deserializing_primary_key` is effective only for the
// deserializing variants
#[test]
fn serde_skip_deserializing_pk_needs_deserializing_variant() {
    for (variant, expected) in [
        (WithSerde::Serialize, false),
        (WithSerde::Deserialize, true),
        (WithSerde::Both, true),
    ] {
        let generated = generate(
            task_schema(),
            Opts {
                with_serde: variant,
                serde_skip_deserializing_primary_key: true,
                ..Default::default()
            },
        );
        let task = generated.file("task.rs");
        let needle = "#[pgorm(primary_key)] #[serde(skip_deserializing)] pub id: i32,";
        if expected {
            assert_contains(task, needle);
        } else {
            assert_not_contains(task, "#[serde(skip_deserializing)]");
        }
    }
}

// [spec:pgorm:sem:codegen.entity.serde.skip/test]    the hidden-column check wins
// for a hidden primary key
#[test]
fn hidden_column_check_wins_for_hidden_pk() {
    let generated = generate(
        vec![keyed_with(
            "task",
            &["_id"],
            vec![
                ColumnDef::new_with_type(Name::runtime("_id"), ColumnType::Integer)
                    .not_null()
                    .to_owned(),
            ],
        )],
        Opts {
            with_serde: WithSerde::Both,
            serde_skip_hidden_column: true,
            serde_skip_deserializing_primary_key: true,
            ..Default::default()
        },
    );

    let task = generated.file("task.rs");
    assert_contains(task, "#[serde(skip)] pub id: i32,");
    assert_not_contains(task, "#[serde(skip_deserializing)]");
}

// [spec:pgorm:sem:codegen.entity.serde.skip/test]    with `WithSerde::None` both
// flags are inert
#[test]
fn serde_skip_flags_are_inert_without_serde() {
    let generated = generate(
        task_schema(),
        Opts {
            with_serde: WithSerde::None,
            serde_skip_hidden_column: true,
            serde_skip_deserializing_primary_key: true,
            ..Default::default()
        },
    );

    let task = generated.file("task.rs");
    assert_not_contains(task, "#[serde(skip)]");
    assert_not_contains(task, "#[serde(skip_deserializing)]");
}

// [spec:pgorm:sem:codegen.entity.serde.derives+3/test]    `extra_derive` is the one
// producer of the serde derive fragment
#[test]
fn with_serde_extra_derive_produces_comma_fragment() {
    assert_eq!(WithSerde::None.extra_derive().to_string(), "");
    assert_eq!(
        norm(&WithSerde::Serialize.extra_derive().to_string()),
        norm(", Serialize")
    );
    assert_eq!(
        norm(&WithSerde::Deserialize.extra_derive().to_string()),
        norm(", Deserialize")
    );
    assert_eq!(
        norm(&WithSerde::Both.extra_derive().to_string()),
        norm(", Serialize, Deserialize")
    );
}

// [spec:pgorm:sem:codegen.entity.serde.derives+3/test]    both Model writers splice
// the fragment after the base derives and the conditional `Eq`, before
// `model_extra_derives`
#[test]
fn serde_derives_are_spliced_after_the_eq_slot() {
    // compact, `Eq` present
    let compact = generate(
        task_schema(),
        Opts {
            with_serde: WithSerde::Both,
            ..Default::default()
        },
    );
    assert_contains(
        compact.file("task.rs"),
        "#[derive(Clone, Debug, PartialEq, Entity, Eq, Serialize, Deserialize)]",
    );

    // extra derives come after the fragment
    let extra = generate(
        task_schema(),
        Opts {
            with_serde: WithSerde::Both,
            model_extra_derives: vec!["Default".to_owned()],
            ..Default::default()
        },
    );
    assert_contains(
        extra.file("task.rs"),
        "#[derive(Clone, Debug, PartialEq, Entity, Eq, Serialize, Deserialize, Default)]",
    );

    // `Eq` suppressed by a float column: the fragment still occupies the slot
    let floaty = generate(
        vec![keyed_with(
            "task",
            &["id"],
            vec![serial("id"), typed("ratio", ColumnType::Float)],
        )],
        Opts {
            with_serde: WithSerde::Both,
            model_extra_derives: vec!["Default".to_owned()],
            ..Default::default()
        },
    );
    assert_contains(
        floaty.file("task.rs"),
        "#[derive(Clone, Debug, PartialEq, Entity, Serialize, Deserialize, Default)]",
    );
}

// [spec:pgorm:sem:codegen.entity.serde.derives+3/test]    the same fragment is
// appended to generated active enums after the base derives and optional `Copy`,
// and always travels with the serde import
#[test]
fn serde_derives_reach_enums_after_copy_with_import() {
    let generated = generate(
        task_schema(),
        Opts {
            with_serde: WithSerde::Both,
            with_copy_enums: true,
            enum_extra_derives: vec!["Hash".to_owned()],
            ..Default::default()
        },
    );
    let enums = generated.file("pgorm_active_enums.rs");

    assert_contains(
        enums,
        "#[derive(Debug, Clone, PartialEq, Eq, ActiveEnum, Copy, Serialize, Deserialize, Hash)]",
    );
    // the fragment and the import always travel together
    assert_contains(enums, "use serde::{Deserialize, Serialize};");
    assert_contains(
        generated.file("task.rs"),
        "use serde::{Deserialize, Serialize};",
    );
}
