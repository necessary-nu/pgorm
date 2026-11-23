//! pgorm targets one PostgreSQL release, 19, and no cargo feature names a
//! release (`sql.target` in docs/spec/sql-ast.md): a feature for one would let
//! a crate elsewhere in a build's graph move every crate's target with it.
//!
//! No database: the checks read the manifests.

use std::collections::BTreeMap;
use std::path::Path;

use toml::{Table, Value};

fn manifest(directory: &str) -> Table {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(directory)
        .join("Cargo.toml");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .parse::<Table>()
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Every crate built from this checkout as part of pgorm: the workspace's
/// members and the two bindings, keyed by directory.
fn crates() -> BTreeMap<String, Table> {
    let root = manifest(".");
    let members = root["workspace"]["members"]
        .as_array()
        .expect("the root declares its members");
    members
        .iter()
        .map(|member| member.as_str().expect("a member is a path").to_owned())
        .chain(["pgorm-python".to_owned(), "pgorm-napi".to_owned()])
        .map(|directory| {
            let table = manifest(&directory);
            (directory, table)
        })
        .collect()
}

/// No crate built from this checkout declares a feature named for a
/// PostgreSQL release, an earlier one or a later one.
// [spec:pgorm:req:sql.target+4/test]    no feature names a release
#[test]
fn no_crate_declares_a_release_feature() {
    for (directory, manifest) in crates() {
        let Some(features) = manifest.get("features").and_then(Value::as_table) else {
            continue;
        };
        let named: Vec<&String> = features
            .keys()
            .filter(|name| {
                name.strip_prefix("pg-")
                    .is_some_and(|rest| rest.parse::<u32>().is_ok())
            })
            .collect();
        assert!(named.is_empty(), "{directory} declares {named:?}");
    }
}
