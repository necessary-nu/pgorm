//! Which PostgreSQL release a build targets, and how the `pg-19` feature
//! travels between the crates that carry it.
//!
//! PostgreSQL 18 is the floor and the default, so no feature names it; `pg-19`
//! targets 19 (`sql.target` in docs/spec/sql-ast.md). The typed surface is
//! pgorm-query's, so the feature is declared there and forwarded by the crates
//! a build turns it on through. A forwarding edge that went missing would
//! leave pgorm built for 19 over a pgorm-query built for 18, and nothing else
//! would notice until a 19-only item failed to resolve.
//!
//! No database: the checks read the build's own constant and the manifests.

use std::collections::BTreeMap;
use std::path::Path;

use toml::{Table, Value};

/// The crates that carry `pg-19`, relative to the checkout root, each with what
/// the feature turns on: pgorm-query declares it, pgorm forwards it to
/// pgorm-query, and the two bindings, pgorm-python and pgorm-napi, each a
/// workspace of its own, forward it to pgorm.
const CARRIERS: [(&str, &[&str]); 4] = [
    ("pgorm-query", &[]),
    (".", &["pgorm-query/pg-19"]),
    ("pgorm-python", &["pgorm/pg-19"]),
    ("pgorm-napi", &["pgorm/pg-19"]),
];

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

fn feature<'a>(manifest: &'a Table, name: &str) -> Option<&'a Value> {
    manifest.get("features")?.get(name)
}

/// The build's constant names the release pgorm's own feature asks for, so
/// pgorm's `pg-19` reached pgorm-query in this build.
// [spec:pgorm:req:sql.target+2/test]    the constant follows the feature through
// pgorm into pgorm-query
#[test]
fn the_target_follows_the_feature() {
    let expected = if cfg!(feature = "pg-19") { 19 } else { 18 };
    assert_eq!(pgorm::pgorm_query::POSTGRES_TARGET, expected);
}

/// pgorm-query declares `pg-19`, pgorm and the bindings forward it to the crate
/// beneath them and to nothing else, no other crate carries it, and none
/// declares a feature for 18, the default.
// [spec:pgorm:req:bindings.postgres-target/test]    each binding forwards it to pgorm
// [spec:pgorm:req:sql.target+2/test]    the feature's declarations and forwarding
// edges, and the absence of a pg-18 feature
#[test]
fn pg_19_is_declared_once_and_forwarded() {
    let crates = crates();
    for (directory, manifest) in &crates {
        assert!(
            feature(manifest, "pg-18").is_none(),
            "{directory} declares pg-18, but 18 is the default and has no feature"
        );
        let forwards = feature(manifest, "pg-19").map(|value| {
            value
                .as_array()
                .unwrap_or_else(|| panic!("{directory}: pg-19 is a list"))
                .iter()
                .map(|edge| edge.as_str().expect("a feature edge is a string"))
                .collect::<Vec<_>>()
        });
        let expected = CARRIERS
            .iter()
            .find(|(carrier, _)| carrier == directory)
            .map(|(_, edges)| edges.to_vec());
        assert_eq!(forwards, expected, "{directory}'s pg-19");
    }
    for (carrier, _) in CARRIERS {
        assert!(crates.contains_key(carrier), "{carrier} is not built here");
    }
}
