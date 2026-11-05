//! pgorm's Cargo features: each changes the build, and the guide lists every
//! one a program turns on.
//!
//! No database: the checks read the manifest, the sources and the guide.

use std::path::{Path, PathBuf};

use toml::Table;

/// Declared for pgorm's own tests and hidden from its documentation, so the
/// guide does not list it.
const HIDDEN: &[&str] = &["tests-cfg"];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every feature the manifest declares but `default`, with what it turns on.
fn features() -> Vec<(String, Vec<String>)> {
    let path = root().join("Cargo.toml");
    let manifest = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .parse::<Table>()
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    manifest["features"]
        .as_table()
        .expect("pgorm declares features")
        .iter()
        .filter(|(name, _)| name.as_str() != "default")
        .map(|(name, enables)| {
            let enables = enables
                .as_array()
                .expect("a feature is a list")
                .iter()
                .map(|item| item.as_str().expect("a feature item is text").to_owned())
                .collect();
            (name.clone(), enables)
        })
        .collect()
}

fn sources(directory: &Path, into: &mut String) {
    for entry in std::fs::read_dir(directory).expect("src/ is readable") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            sources(&path, into);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            into.push_str(&std::fs::read_to_string(&path).expect("a source file is readable"));
        }
    }
}

// [spec:pgorm:req:guide.features/test]    every declared feature forwards to a dependency or is read by a cfg
#[test]
fn every_feature_changes_the_build() {
    let mut source = String::new();
    sources(&root().join("src"), &mut source);
    let inert: Vec<String> = features()
        .into_iter()
        .filter(|(name, enables)| {
            enables.is_empty() && !source.contains(&format!("feature = \"{name}\""))
        })
        .map(|(name, _)| name)
        .collect();
    assert!(
        inert.is_empty(),
        "features that turn nothing on and that no cfg in src/ reads: {inert:?}"
    );
}

// [spec:pgorm:req:guide.features/test]    the getting-started chapter's table lists exactly the features a program turns on
#[test]
fn the_guide_lists_every_feature() {
    let path = root().join("docs/guide/01-getting-started.md");
    let chapter = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let mut listed: Vec<&str> = chapter
        .lines()
        .skip_while(|line| !line.starts_with("| Feature |"))
        .skip(2)
        .take_while(|line| line.starts_with('|'))
        .map(|line| {
            line.trim_start_matches("| `")
                .split('`')
                .next()
                .expect("a row names its feature")
        })
        .collect();
    listed.sort_unstable();
    let declared = features();
    let mut expected: Vec<&str> = declared
        .iter()
        .map(|(name, _)| name.as_str())
        .filter(|name| !HIDDEN.contains(name))
        .collect();
    expected.sort_unstable();
    assert_eq!(listed, expected);
}
