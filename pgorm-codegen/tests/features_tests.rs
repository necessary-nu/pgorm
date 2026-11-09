//! The Rust writer is the `rust` feature's, and the bindings build without
//! it: what the manifests declare, which a build without the feature then
//! holds the crate to.

use std::path::Path;

fn manifest(path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    std::fs::read_to_string(&path).expect("the manifest is readable")
}

/// The line of `manifest` declaring the dependency `name`.
fn dependency<'a>(manifest: &'a str, name: &str) -> &'a str {
    manifest
        .lines()
        .find(|line| line.starts_with(&format!("{name} = ")))
        .unwrap_or_else(|| panic!("{name} is a dependency"))
}

// [spec:pgorm:req:codegen.features/test]    syn, quote, prettyplease and proc-macro2 are optional and the default rust feature's alone, and pgorm-python and pgorm-napi take the crate without it
#[test]
fn bindings_build_without_the_rust_writer() {
    let own = manifest("Cargo.toml");
    let features = own
        .split("[features]")
        .nth(1)
        .and_then(|rest| rest.split("\n[").next())
        .expect("the crate declares features");
    assert!(features.contains("default = [\"rust\"]"), "{features}");
    for name in ["syn", "quote", "prettyplease", "proc-macro2"] {
        assert!(dependency(&own, name).contains("optional = true"), "{name}");
        assert!(features.contains(&format!("\"dep:{name}\"")), "{name}");
    }
    for binding in ["../pgorm-python/Cargo.toml", "../pgorm-napi/Cargo.toml"] {
        let held = manifest(binding);
        let line = dependency(&held, "pgorm-codegen");
        assert!(
            line.contains("default-features = false"),
            "{binding}: {line}"
        );
    }
}

// [spec:pgorm:req:codegen.features/test]    a name only Rust's gate refuses, a column no Rust field can be named, stops the entities and not the declared models
#[test]
fn rusts_names_hold_the_entities_alone() {
    let ddl = "CREATE TABLE t (id int PRIMARY KEY, \"1st\" int);";
    let refused = pgorm_codegen::entities_from_sql(ddl, Default::default())
        .expect_err("a column no Rust field can be named");
    assert!(
        refused.to_string().contains("not a valid Rust identifier"),
        "{refused}"
    );
    let written = pgorm_codegen::python_models_from_sql(ddl, Default::default())
        .expect("a declared model names its field by its column");
    assert!(
        written.files[0]
            .content
            .contains("\"1st\": Column(\"i32\", nullable=True)")
    );
}
