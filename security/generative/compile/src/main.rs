//! Run `pgorm_codegen::entities_from_sql` over generated DDL and report what it did.
//!
//! The compile suite needs generated entity source it can hand to a compiler,
//! and it needs to tell a *generator* refusal apart from a *compile* refusal.
//! Those are different claims about the library: `entities_from_sql` returning
//! `Err` is codegen doing its job at its own boundary, while a rejected build
//! is rustc doing its job at the next one. Collapsing them would let a broken
//! generator masquerade as a well-defended type system.
//!
//! One process handles a whole batch: the driver is a path dependency on
//! `pgorm-codegen`, so starting it again per case would pay process startup
//! for nothing.

use std::io::{Read, Write};

use pgorm_codegen::{EntityWriterOptions, WithSerde, entities_from_sql};
use serde::{Deserialize, Serialize};

/// One generation request: DDL text plus every `EntityWriterOptions` field.
///
/// Options arrive already spelled as the library spells them. The suite varies
/// them; the driver does not invent defaults beyond `serde`'s, so a field the
/// suite forgot reads as the library's own default rather than as a value the
/// driver chose.
#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    sql: String,
    #[serde(default)]
    with_serde: String,
    #[serde(default)]
    with_copy_enums: bool,
    #[serde(default)]
    schema_name: Option<String>,
    #[serde(default)]
    lib: bool,
    #[serde(default)]
    serde_skip_deserializing_primary_key: bool,
    #[serde(default)]
    serde_skip_hidden_column: bool,
    #[serde(default)]
    model_extra_derives: Vec<String>,
    #[serde(default)]
    model_extra_attributes: Vec<String>,
    #[serde(default)]
    enum_extra_derives: Vec<String>,
    #[serde(default)]
    enum_extra_attributes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Request {
    cases: Vec<Case>,
}

#[derive(Debug, Serialize)]
struct File {
    name: String,
    content: String,
}

/// What generation did, named so the suite never has to infer it from an exit
/// status: `generated` carries source to compile, `generator-error` carries the
/// library's own refusal and nothing to compile.
#[derive(Debug, Serialize)]
struct Outcome {
    id: String,
    outcome: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files: Vec<File>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Debug, Serialize)]
struct Response {
    cases: Vec<Outcome>,
}

/// An unreadable option spelling is the driver's fault, not the library's, so
/// it is reported as its own outcome rather than as a generation refusal.
fn serde_mode(text: &str) -> Result<WithSerde, String> {
    Ok(match text {
        "" | "none" => WithSerde::None,
        "serialize" => WithSerde::Serialize,
        "deserialize" => WithSerde::Deserialize,
        "both" => WithSerde::Both,
        other => return Err(format!("unknown with_serde spelling `{other}`")),
    })
}

fn options(case: &Case) -> Result<EntityWriterOptions, String> {
    Ok(EntityWriterOptions {
        with_serde: serde_mode(&case.with_serde)?,
        with_copy_enums: case.with_copy_enums,
        schema_name: case.schema_name.clone(),
        lib: case.lib,
        serde_skip_deserializing_primary_key: case.serde_skip_deserializing_primary_key,
        serde_skip_hidden_column: case.serde_skip_hidden_column,
        model_extra_derives: case.model_extra_derives.clone(),
        model_extra_attributes: case.model_extra_attributes.clone(),
        enum_extra_derives: case.enum_extra_derives.clone(),
        enum_extra_attributes: case.enum_extra_attributes.clone(),
    })
}

fn generate(case: &Case) -> Outcome {
    let requested = match options(case) {
        Ok(value) => value,
        Err(message) => {
            return Outcome {
                id: case.id.clone(),
                outcome: "driver-error",
                files: Vec::new(),
                message: Some(message),
            };
        }
    };
    match entities_from_sql(&case.sql, requested) {
        Ok(output) => Outcome {
            id: case.id.clone(),
            outcome: "generated",
            files: output
                .files
                .into_iter()
                .map(|file| File {
                    name: file.name,
                    content: file.content,
                })
                .collect(),
            message: None,
        },
        Err(error) => Outcome {
            id: case.id.clone(),
            outcome: "generator-error",
            files: Vec::new(),
            message: Some(error.to_string()),
        },
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let request: Request = serde_json::from_str(&input)?;
    let response = Response {
        cases: request.cases.iter().map(generate).collect(),
    };
    let mut out = std::io::stdout().lock();
    out.write_all(serde_json::to_string(&response)?.as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(sql: &str) -> Case {
        serde_json::from_value(serde_json::json!({"id": "c0", "sql": sql}))
            .expect("the fixture names only declared fields")
    }

    #[test]
    fn known_option_spellings_map_to_library_values() {
        assert!(matches!(serde_mode(""), Ok(WithSerde::None)));
        assert!(matches!(serde_mode("both"), Ok(WithSerde::Both)));
    }

    #[test]
    fn an_unknown_spelling_is_the_drivers_fault() {
        // Not a generation refusal: the suite asked for something the driver
        // cannot express, and blaming codegen for it would hide a harness bug.
        let mut wrong = case("CREATE TABLE t (id integer PRIMARY KEY);");
        wrong.with_serde = "yes".to_owned();
        let outcome = generate(&wrong);
        assert_eq!(outcome.outcome, "driver-error");
        assert!(outcome.files.is_empty());
        assert!(outcome.message.expect("a reason").contains("with_serde"));
    }

    #[test]
    fn codegen_refusing_is_reported_as_its_own() {
        let outcome = generate(&case("CREATE VIEW v AS SELECT 1;"));
        assert_eq!(outcome.outcome, "generator-error");
        assert!(outcome.files.is_empty());
        assert!(outcome.message.is_some());
    }

    #[test]
    fn generated_source_arrives_with_its_files() {
        let outcome = generate(&case("CREATE TABLE t (id integer PRIMARY KEY);"));
        assert_eq!(outcome.outcome, "generated");
        assert!(outcome.message.is_none());
        assert!(outcome.files.iter().any(|file| file.name.contains('t')));
        assert!(!outcome.files.is_empty());
    }
}
