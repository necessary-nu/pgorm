//! The embedded engine is an opt-in driver dependency, including in the
//! bindings' detached workspaces. Lockfiles must preserve one codec identity.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use toml::{Table, Value};

const DRIVER: &str = "crates/pgorm-driver/Cargo.toml";
const BENCHMARK: &str = "benches/native/Cargo.toml";
const ENGINE: [&str; 2] = ["turso_pg", "turso_core"];
const REPOSITORY: &str = "https://github.com/tursodatabase/turso.git";
const FEATURE_EDGES: &[(&str, &[&str])] = &[
    ("Cargo.toml", &["pgorm-driver/turso", "pgorm-pool/turso"]),
    ("pgorm-pool/Cargo.toml", &["pgorm-driver/turso"]),
    (
        "pgorm-bindings/Cargo.toml",
        &["pgorm/turso", "pgorm-driver/turso"],
    ),
    (
        "pgorm-python/Cargo.toml",
        &["pgorm/turso", "pgorm-bindings/turso", "pgorm-driver/turso"],
    ),
    (
        "pgorm-napi/Cargo.toml",
        &["pgorm/turso", "pgorm-bindings/turso", "pgorm-driver/turso"],
    ),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> Table {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .parse()
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn tracked(suffix: &str) -> Vec<PathBuf> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root())
        .output()
        .expect("git enumerates the checkout's manifests and committed locks");
    assert!(output.status.success(), "git ls-files failed");
    String::from_utf8(output.stdout)
        .expect("checkout paths are UTF-8")
        .split('\0')
        .filter(|path| path.ends_with(suffix))
        .map(|path| root().join(path))
        .collect()
}

fn dependencies<'a>(table: &'a Table, into: &mut Vec<(&'a str, &'a Value)>) {
    for (key, value) in table {
        let Some(nested) = value.as_table() else {
            continue;
        };
        if matches!(
            key.as_str(),
            "dependencies" | "build-dependencies" | "dev-dependencies"
        ) {
            into.extend(nested.iter().map(|(key, value)| {
                (
                    value.get("package").and_then(Value::as_str).unwrap_or(key),
                    value,
                )
            }));
        } else {
            dependencies(nested, into);
        }
    }
}

fn pin() -> String {
    let driver = read(&root().join(DRIVER));
    let dependencies = driver["dependencies"].as_table().unwrap();
    let rev = dependencies[ENGINE[0]]["rev"].as_str().unwrap();
    assert!(rev.len() == 40 && rev.bytes().all(|byte| byte.is_ascii_hexdigit()));
    for name in ENGINE {
        let dependency = &dependencies[name];
        assert_eq!(dependency["git"].as_str(), Some(REPOSITORY), "{name}");
        assert_eq!(dependency["rev"].as_str(), Some(rev), "{name}");
        assert_eq!(dependency["optional"].as_bool(), Some(true), "{name}");
        for forbidden in ["version", "branch", "tag", "path", "registry"] {
            assert!(dependency.get(forbidden).is_none(), "{name}: {forbidden}");
        }
    }
    rev.to_owned()
}

// [spec:pgorm:req:turso.feature/test]
#[test]
fn engine_dependencies_stay_in_driver_and_benchmarks() {
    let revision = pin();
    let benchmark = read(&root().join(BENCHMARK));
    assert!(benchmark["workspace"].is_table());
    assert_eq!(benchmark["package"]["publish"].as_bool(), Some(false));
    assert_eq!(
        read(&root().join("Cargo.toml"))["package"]["autobenches"].as_bool(),
        Some(false)
    );
    for path in tracked("Cargo.toml") {
        let manifest = read(&path);
        let mut declared = Vec::new();
        dependencies(&manifest, &mut declared);
        for (name, dependency) in declared {
            assert_ne!(
                name,
                "pgorm-native-performance",
                "benchmark dependency in {}",
                path.display()
            );
            if ENGINE.contains(&name) {
                assert!(
                    path == root().join(DRIVER) || path == root().join(BENCHMARK),
                    "engine dependency {name} in {}",
                    path.display()
                );
                assert_eq!(dependency["git"].as_str(), Some(REPOSITORY), "{name}");
                assert_eq!(
                    dependency["rev"].as_str(),
                    Some(revision.as_str()),
                    "{name}"
                );
            }
        }
    }
    let driver = read(&root().join(DRIVER));
    let feature = driver["features"]["turso"].as_array().unwrap();
    for name in ENGINE {
        assert!(
            feature
                .iter()
                .any(|item| item.as_str() == Some(&format!("dep:{name}")))
        );
    }
}

// [spec:pgorm:req:turso.feature/test]
#[test]
fn native_feature_propagates_without_entering_defaults() {
    for &(manifest, expected) in FEATURE_EDGES {
        let manifest = read(&root().join(manifest));
        let feature = manifest["features"]["turso"].as_array().unwrap();
        for edge in expected {
            assert!(
                feature.iter().any(|item| item.as_str() == Some(edge)),
                "missing {edge}"
            );
        }
    }
    for name in std::iter::once(DRIVER).chain(FEATURE_EDGES.iter().map(|(path, _)| *path)) {
        let manifest = read(&root().join(name));
        let features = manifest["features"].as_table().unwrap();
        let mut pending = vec!["default"];
        let mut visited = BTreeSet::new();
        while let Some(feature) = pending.pop() {
            if !visited.insert(feature) {
                continue;
            }
            assert!(
                feature != "turso"
                    && !feature.ends_with("/turso")
                    && !ENGINE
                        .iter()
                        .any(|name| feature == *name || feature == format!("dep:{name}")),
                "{name} enables {feature} by default",
            );
            if let Some(items) = features.get(feature).and_then(Value::as_array) {
                pending.extend(items.iter().filter_map(Value::as_str));
            }
        }
        let mut declared = Vec::new();
        dependencies(&manifest, &mut declared);
        for (dependency, spec) in declared {
            let enabled = spec.get("features").and_then(Value::as_array);
            assert!(
                !enabled
                    .into_iter()
                    .flatten()
                    .any(|item| item.as_str() == Some("turso")),
                "{name} enables {dependency}/turso unconditionally",
            );
        }
    }
}

// [spec:pgorm:req:turso.feature/test]
#[test]
fn driver_lockfiles_share_engine_and_codec_identities() {
    let expected = format!("git+{REPOSITORY}?rev={0}#{0}", pin());
    let mut locks = tracked("Cargo.lock");
    // The root workspace's generated lock is ignored, but it must agree too.
    let root_lock = root().join("Cargo.lock");
    if root_lock.exists() && !locks.contains(&root_lock) {
        locks.push(root_lock);
    }
    let mut checked = 0;
    let mut native_checked = 0;
    for path in locks {
        let lock = read(&path);
        let packages = lock["package"].as_array().unwrap();
        if !packages
            .iter()
            .any(|package| package["name"].as_str() == Some("pgorm-driver"))
        {
            continue;
        }
        checked += 1;
        let has_engine = packages.iter().any(|package| {
            package["name"]
                .as_str()
                .is_some_and(|name| ENGINE.contains(&name))
        });
        native_checked += usize::from(has_engine);
        for name in ENGINE
            .into_iter()
            .chain(["postgres-types", "postgres-protocol"])
        {
            if ENGINE.contains(&name) && !has_engine {
                continue;
            }
            let matching: Vec<_> = packages
                .iter()
                .filter(|package| package["name"].as_str() == Some(name))
                .collect();
            assert_eq!(
                matching.len(),
                1,
                "{} must resolve one {name}",
                path.display()
            );
            let source = matching[0]["source"].as_str().unwrap();
            if ENGINE.contains(&name) {
                assert_eq!(source, expected, "{}: {name}", path.display());
            } else {
                assert!(
                    source.starts_with("registry+"),
                    "{}: {name}",
                    path.display()
                );
            }
        }
        for package in packages {
            if let Some(source) = package.get("source").and_then(Value::as_str)
                && source.starts_with(&format!("git+{REPOSITORY}"))
            {
                assert_eq!(
                    source,
                    expected,
                    "{} mixes engine revisions",
                    path.display()
                );
            }
        }
    }
    assert!(checked > 0, "no driver lockfile was checked");
    assert!(
        native_checked > 0,
        "no lockfile resolved the optional engine"
    );
}
