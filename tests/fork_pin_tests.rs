//! pgorm links one prqlc and one pg_query: the forks its own manifest names,
//! each at one revision.
//!
//! The root crate depends on two forks through git dependencies — the
//! necessary-nu fork of prqlc, which compiles `pgorm::pipeline` and `prql!`,
//! and the necessary-nu fork of pg_query.rs, which carries PostgreSQL 18's
//! grammar ahead of upstream's release — and a dependency travels with pgorm
//! into every crate that depends on it: the crates here that declare their own
//! `[workspace]` (`pgorm-python` and `pgorm-napi`, so that each binding's
//! configuration stays out of ordinary Rust builds
//! ([spec:pgorm:req:bindings.optional]), and the campaign's bridge, codegen
//! runner, replay harness, sqlmap adapter and standalone reproducers, so that
//! their dependencies stay out of an ordinary pgorm
//! build), the compile-suite crates the campaign generates at run time, and
//! downstream consumers.
//!
//! The prqlc fork used to be applied by a `[patch.crates-io]` table instead,
//! and cargo reads a patch from the root of the workspace being built and
//! nowhere else. Every detached workspace therefore had to repeat the table,
//! and one that did not silently linked stock prqlc and compiled different SQL
//! from the one every pgorm test exercises. That hid three fork commits from
//! every campaign run of 2026-09-15; it then turned out eight more detached
//! workspaces had never repeated the table, under a version of this file that
//! listed the two it knew about; and finally the compile suite's generated
//! crates, which no committed manifest describes, linked stock prqlc on every
//! run. Each fix covered the instances it could see. Making the fork the
//! dependency removes the class: nothing has to be repeated, so nothing can be
//! forgotten. The pg_query fork was adopted the same way from the start, and is
//! held here by the same checks, because a detached workspace resolving stock
//! pg_query would parse with PostgreSQL 17's grammar while every pgorm test
//! parses with 18's.
//!
//! These tests hold the invariant from both ends, for each fork. The root
//! declares the fork at a full commit, every other declaration names the same
//! one, and nothing in the checkout patches it — a patch would be a second
//! source of truth, and the one that reaches some builds and not others. And
//! every lockfile in the checkout that resolves the fork's crates resolves them
//! from the fork at that revision, which is what a `--locked` build reads. The
//! generated compile crates are held to the same thing by the campaign's own
//! suite (`security/generative/tests/test_compile_lockfile.py`), because they
//! exist only while the campaign runs.
//!
//! No database: every check reads version-controlled files.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use toml::{Table, Value};

/// A forked dependency the root crate declares by git revision.
struct Fork {
    /// The key the root's `[dependencies]` declares it under.
    dependency: &'static str,
    /// The repository every declaration has to name. Moving to another
    /// repository is a decision, so it is written here rather than read back
    /// from the manifest under test.
    repository: &'static str,
    /// The crates the fork's repository provides, held to one source so that
    /// declaring any of them directly cannot split the dependency in two.
    crates: &'static [&'static str],
    /// The manifests that have to declare it, relative to the checkout root.
    declarers: &'static [&'static str],
    /// Detached crates discovery has to find linking it. These are known
    /// instances, not the set: the set is whatever discovery finds.
    detached: &'static [&'static str],
    /// Detached crates discovery has to find not linking it.
    not_linking: &'static [&'static str],
}

/// The detached crates that reach pgorm's runtime, and so both forks.
const RUNTIME_DETACHED: [&str; 8] = [
    "pgorm-python",
    "pgorm-napi",
    "security/generative/bridge",
    "security/sqlmap/adapter",
    "security/generative/replay",
    "pgorm-python/tests/application-binding",
    "pgorm-python/tests/codegen-entities",
    "security/generative/findings/set-precedence",
];

/// The PRQL compiler. Only `prqlc` is declared anywhere today;
/// `prqlc-parser` comes with it.
const PRQLC: Fork = Fork {
    dependency: "prqlc",
    repository: "https://github.com/necessary-nu/prql.git",
    crates: &["prqlc", "prqlc-parser"],
    declarers: &["Cargo.toml", "pgorm-sql-check/Cargo.toml"],
    detached: &RUNTIME_DETACHED,
    // The codegen runner reaches pgorm only through pgorm-codegen's
    // development dependency, so it compiles no prqlc.
    not_linking: &["security/generative/compile"],
};

/// libpg_query's Rust binding, on PostgreSQL 18's grammar.
const PG_QUERY: Fork = Fork {
    dependency: "pg_query",
    repository: "https://github.com/necessary-nu/pg_query.rs.git",
    crates: &["pg_query"],
    declarers: &[
        "Cargo.toml",
        "pgorm-sql-check/Cargo.toml",
        "pgorm-codegen/Cargo.toml",
        "pgorm-query/Cargo.toml",
    ],
    // pgorm-codegen parses DDL with pg_query itself, so the codegen runner
    // links it where it links no prqlc.
    detached: &[
        "pgorm-python",
        "pgorm-napi",
        "security/generative/bridge",
        "security/sqlmap/adapter",
        "security/generative/replay",
        "pgorm-python/tests/application-binding",
        "pgorm-python/tests/codegen-entities",
        "security/generative/findings/set-precedence",
        "security/generative/compile",
    ],
    not_linking: &[],
};

const FORKS: [&Fork; 2] = [&PRQLC, &PG_QUERY];

fn repository_root() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    std::fs::canonicalize(&root).unwrap_or(root)
}

fn parse(path: &Path) -> Table {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => panic!("{} must be readable: {error}", path.display()),
    };
    match text.parse() {
        Ok(table) => table,
        Err(error) => panic!("{} must be TOML: {error}", path.display()),
    }
}

/// The files in the checkout that decide which fork revision a build resolves.
struct Checkout {
    root: PathBuf,
    /// Every `Cargo.toml`.
    manifests: BTreeMap<PathBuf, Table>,
    /// Every `.cargo/config.toml` and legacy `.cargo/config`, which can carry
    /// a `[patch]` table of their own.
    configs: BTreeMap<PathBuf, Table>,
    /// Every `Cargo.lock`.
    lockfiles: BTreeMap<PathBuf, Table>,
}

impl Checkout {
    /// Walk the checkout.
    ///
    /// `target/` holds an unpacked manifest for every registry dependency and
    /// is a build output besides, so it is skipped rather than walked; so is
    /// any dotted directory, `.git` above all, except that a `.cargo`
    /// directory is read for its configuration.
    fn read() -> Self {
        let root = repository_root();
        let mut checkout = Self {
            root: root.clone(),
            manifests: BTreeMap::new(),
            configs: BTreeMap::new(),
            lockfiles: BTreeMap::new(),
        };
        let mut pending = vec![root];
        while let Some(directory) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if path.is_dir() {
                    if name == ".cargo" {
                        for config in ["config.toml", "config"] {
                            let file = path.join(config);
                            if file.is_file() {
                                checkout.configs.insert(file.clone(), parse(&file));
                            }
                        }
                    } else if name != "target" && !name.starts_with('.') {
                        pending.push(path);
                    }
                } else if name == "Cargo.toml" {
                    checkout.manifests.insert(path.clone(), parse(&path));
                } else if name == "Cargo.lock" {
                    checkout.lockfiles.insert(path.clone(), parse(&path));
                }
            }
        }
        checkout
    }

    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }

    fn root_manifest(&self) -> &Table {
        match self.manifests.get(&self.root.join("Cargo.toml")) {
            Some(manifest) => manifest,
            None => panic!("the checkout root has a Cargo.toml"),
        }
    }
}

/// One dependency edge as a manifest declares it.
struct Dependency<'a> {
    /// The table the edge is declared in, for messages.
    table: String,
    /// The key it is declared under.
    key: &'a str,
    /// The crate it resolves to: the key, or the `package` it renames.
    crate_name: &'a str,
    spec: &'a Value,
    /// Development dependencies are linked by the declaring crate's own
    /// tests, and by nothing that depends on it.
    development: bool,
}

impl Dependency<'_> {
    fn field(&self, name: &str) -> Option<&str> {
        self.spec.get(name).and_then(Value::as_str)
    }
}

/// The dependency tables a manifest section holds, named for messages.
fn dependency_tables<'a>(
    prefix: &str,
    holder: &'a Table,
    into: &mut Vec<(String, &'a Table, bool)>,
) {
    for (key, development) in [
        ("dependencies", false),
        ("build-dependencies", false),
        ("build_dependencies", false),
        ("dev-dependencies", true),
        ("dev_dependencies", true),
    ] {
        if let Some(Value::Table(table)) = holder.get(key) {
            into.push((format!("{prefix}{key}"), table, development));
        }
    }
}

/// Every dependency a manifest declares, in whichever form it is written:
/// TOML is parsed rather than read line by line, so the `[dependencies.name]`
/// table form, a `package` rename and a platform-conditional table are all
/// seen.
fn declared_dependencies(manifest: &Table) -> Vec<Dependency<'_>> {
    let mut tables = Vec::new();
    dependency_tables("", manifest, &mut tables);
    if let Some(Value::Table(targets)) = manifest.get("target") {
        for (platform, holder) in targets {
            if let Value::Table(holder) = holder {
                dependency_tables(&format!("target.{platform}."), holder, &mut tables);
            }
        }
    }
    if let Some(Value::Table(workspace)) = manifest.get("workspace") {
        dependency_tables("workspace.", workspace, &mut tables);
    }
    tables
        .into_iter()
        .flat_map(|(table, entries, development)| {
            entries.iter().map(move |(key, spec)| Dependency {
                table: table.clone(),
                key,
                crate_name: spec.get("package").and_then(Value::as_str).unwrap_or(key),
                spec,
                development,
            })
        })
        .collect()
}

/// The git URL and revision the root's own dependency on `fork` names.
///
/// A full commit and nothing else: a branch or tag moves under the lockfile,
/// and a `version` beside the `git` key is what `cargo publish` would keep —
/// it would hand every crates.io consumer the stock crate while this checkout
/// kept building the fork, which is the split this file exists to prevent.
fn pinned(checkout: &Checkout, fork: &Fork) -> (String, String) {
    let name = fork.dependency;
    let manifest = checkout.root_manifest();
    let declared: Vec<_> = declared_dependencies(manifest)
        .into_iter()
        .filter(|dependency| dependency.table == "dependencies" && dependency.crate_name == name)
        .collect();
    assert_eq!(
        declared.len(),
        1,
        "the root crate declares {name} exactly once in [dependencies]"
    );
    let dependency = &declared[0];
    let git = dependency
        .field("git")
        .unwrap_or_else(|| panic!("the root's {name} dependency is a git dependency"));
    let rev = dependency
        .field("rev")
        .unwrap_or_else(|| panic!("the root's {name} dependency names a revision"));
    assert_eq!(
        git, fork.repository,
        "the root depends on {name} from the fork"
    );
    assert!(
        rev.len() == 40 && rev.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "the root's {name} revision is a full commit, not {rev}"
    );
    for key in ["version", "branch", "tag", "path", "registry"] {
        assert!(
            dependency.spec.get(key).is_none(),
            "the root's {name} dependency carries `{key}` beside its revision"
        );
    }
    (git.to_owned(), rev.to_owned())
}

/// The lockfile `source` a dependency on the fork at `rev` resolves to.
fn locked_source(git: &str, rev: &str) -> String {
    format!("git+{git}?rev={rev}#{rev}")
}

/// Every patch or replacement a manifest or cargo configuration applies to a
/// crate from `fork`, spelled for messages.
fn fork_patches(config: &Table, fork: &Fork) -> Vec<String> {
    let mut found = Vec::new();
    if let Some(Value::Table(patch)) = config.get("patch") {
        for (source, entries) in patch {
            let Value::Table(entries) = entries else {
                continue;
            };
            for (key, spec) in entries {
                let name = spec.get("package").and_then(Value::as_str).unwrap_or(key);
                if fork.crates.contains(&name) {
                    found.push(format!("[patch.{source}] {key}"));
                }
            }
        }
    }
    if let Some(Value::Table(replace)) = config.get("replace") {
        for key in replace.keys() {
            let name = key.split(':').next().unwrap_or(key);
            if fork.crates.contains(&name) {
                found.push(format!("[replace] {key}"));
            }
        }
    }
    found
}

/// Whether one manifest's dependencies reach a crate of `fork`.
///
/// `carriers` are the manifests already known to carry the fork into anything
/// that depends on them. `own` asks the question for the crate itself, which
/// links its development dependencies too.
fn reaches(
    fork: &Fork,
    manifest_path: &Path,
    manifest: &Table,
    carriers: &BTreeSet<PathBuf>,
    own: bool,
) -> bool {
    let directory = manifest_path.parent().unwrap_or(Path::new("."));
    declared_dependencies(manifest).iter().any(|dependency| {
        if dependency.development && !own {
            return false;
        }
        if fork.crates.contains(&dependency.crate_name) {
            return true;
        }
        dependency.field("path").is_some_and(|relative| {
            let target = directory.join(relative).join("Cargo.toml");
            std::fs::canonicalize(target).is_ok_and(|resolved| carriers.contains(&resolved))
        })
    })
}

/// The manifests whose compiled dependency graph contains a crate of `fork`.
///
/// A crate reaches the fork by naming it or through a path dependency on a
/// crate that does, so the carrying set is grown to a fixed point first and
/// each manifest then asked about its own graph. The distinction is what keeps
/// `security/generative/compile` out of prqlc's set: it depends on
/// `pgorm-codegen`, whose only edge to `pgorm` is a development dependency, and
/// a development dependency is linked by its declarer's tests and by nothing
/// downstream. It is in pg_query's, through `pgorm-codegen`'s own edge.
fn linking(fork: &Fork, manifests: &BTreeMap<PathBuf, Table>) -> BTreeSet<PathBuf> {
    let mut carriers = BTreeSet::new();
    let mut growing = true;
    while growing {
        growing = false;
        for (path, manifest) in manifests {
            if !carriers.contains(path) && reaches(fork, path, manifest, &carriers, false) {
                carriers.insert(path.clone());
                growing = true;
            }
        }
    }
    manifests
        .iter()
        .filter(|(path, manifest)| reaches(fork, path, manifest, &carriers, true))
        .map(|(path, _)| path.clone())
        .collect()
}

/// The crates that declare their own workspace and link `fork`, as paths
/// relative to the checkout root.
///
/// Discovered rather than listed: a list covers the cases its author had
/// already noticed, which is how eight unpatched workspaces accumulated
/// under a test that claimed to cover them.
fn detached_crates_linking(checkout: &Checkout, fork: &Fork) -> Vec<String> {
    let linking = linking(fork, &checkout.manifests);
    let root_manifest = checkout.root.join("Cargo.toml");
    checkout
        .manifests
        .iter()
        .filter(|(path, manifest)| {
            **path != root_manifest && linking.contains(*path) && manifest.contains_key("workspace")
        })
        .filter_map(|(path, _)| Some(checkout.relative(path.parent()?)))
        .collect()
}

/// Every other check here reads the revision from this declaration, so its
/// own shape is asserted by `pinned` itself.
#[test]
fn the_root_depends_on_each_pinned_fork() {
    let checkout = Checkout::read();
    for fork in FORKS {
        pinned(&checkout, fork);
    }
}

/// A second declaration at another revision would compile a second copy
/// beside the first — `pgorm-sql-macro`'s `prql!` emitting through one
/// compiler and `pgorm::pipeline` through another, or `sql!` accepting a
/// statement with one grammar that the paginator reads with another.
#[test]
fn every_declaration_names_the_roots_revision() {
    let checkout = Checkout::read();
    for fork in FORKS {
        let (git, rev) = pinned(&checkout, fork);
        let mut declaring = BTreeSet::new();
        for (path, manifest) in &checkout.manifests {
            let at = checkout.relative(path);
            for dependency in declared_dependencies(manifest) {
                if !fork.crates.contains(&dependency.crate_name) {
                    continue;
                }
                declaring.insert(at.clone());
                // An inherited declaration is the `[workspace.dependencies]`
                // entry, which is itself one of the declarations checked here.
                if dependency.spec.get("workspace").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                let (table, key) = (&dependency.table, dependency.key);
                assert_eq!(
                    (dependency.field("git"), dependency.field("rev")),
                    (Some(git.as_str()), Some(rev.as_str())),
                    "{at} declares {key} in [{table}] from somewhere other than the \
                     fork at the root's revision, so it would link a second copy"
                );
                for field in ["version", "branch", "tag", "path", "registry"] {
                    assert!(
                        dependency.spec.get(field).is_none(),
                        "{at} declares {key} in [{table}] with `{field}` beside its revision"
                    );
                }
            }
        }
        for declarer in fork.declarers {
            assert!(
                declaring.contains(*declarer),
                "{declarer} declares {}; found {declaring:?}",
                fork.dependency
            );
        }
    }
}

// [spec:pgorm:req:bindings.optional/test]    a separate workspace receives the
// pinned revision through pgorm's own dependency, never through a patch
/// A patch is read only by the workspace that declares it, so one anywhere in
/// the checkout would decide the fork for some builds and not others: a second
/// source of truth, which is the thing the git dependency replaced.
#[test]
fn nothing_in_the_checkout_patches_a_fork() {
    let checkout = Checkout::read();
    let mut patched = Vec::new();
    for fork in FORKS {
        for (path, config) in checkout.manifests.iter().chain(&checkout.configs) {
            for entry in fork_patches(config, fork) {
                patched.push(format!("{}: {entry}", checkout.relative(path)));
            }
        }
    }
    assert!(
        patched.is_empty(),
        "each fork reaches every crate through pgorm's own git dependency; a \
         patch for one is a second source of truth that only its own workspace \
         reads, so delete it (or move the root's dependency, which every crate \
         follows): {patched:?}"
    );
}

// [spec:pgorm:req:bindings.optional/test]    and every lockfile resolves the
// pinned revision, which is what a `--locked` build reads
#[test]
fn every_lockfile_resolves_each_fork_from_its_revision() {
    let checkout = Checkout::read();
    for fork in FORKS {
        let (git, rev) = pinned(&checkout, fork);
        let expected = locked_source(&git, &rev);
        let mut resolving = 0;
        for (path, lock) in &checkout.lockfiles {
            let at = checkout.relative(path);
            let Some(Value::Array(packages)) = lock.get("package") else {
                continue;
            };
            // Every package of the name, not the first: a stock copy resolved
            // beside the fork is the failure, and it would hide behind it.
            let resolved: Vec<_> = packages
                .iter()
                .filter(|package| {
                    package
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| fork.crates.contains(&name))
                })
                .collect();
            if resolved.is_empty() {
                continue;
            }
            resolving += 1;
            for package in resolved {
                let name = package.get("name").and_then(Value::as_str).unwrap_or("?");
                let source = package.get("source").and_then(Value::as_str);
                assert_eq!(
                    source,
                    Some(expected.as_str()),
                    "{at} resolves {name} from {source:?}, not from the fork at the root's revision"
                );
            }
        }
        assert!(
            resolving > 0,
            "no lockfile in the checkout resolves {}, so this checked nothing",
            fork.dependency
        );
    }
}

// [spec:pgorm:req:bindings.optional/test]    a detached workspace linking a fork
// commits the lockfile its `--locked` builds read
#[test]
fn detached_crates_linking_a_fork_commit_lockfiles() {
    let checkout = Checkout::read();
    for fork in FORKS {
        for crate_dir in detached_crates_linking(&checkout, fork) {
            let name = fork.dependency;
            let lock = checkout
                .lockfiles
                .get(&checkout.root.join(&crate_dir).join("Cargo.lock"))
                .unwrap_or_else(|| panic!("{crate_dir} links {name}, so it commits a lockfile"));
            let resolves = lock
                .get("package")
                .and_then(Value::as_array)
                .is_some_and(|packages| {
                    packages
                        .iter()
                        .any(|package| package.get("name").and_then(Value::as_str) == Some(name))
                });
            assert!(
                resolves,
                "{crate_dir} links {name}, so its lockfile resolves it"
            );
        }
    }
}

/// The discovery has to reach past the two crates the first listed version
/// knew about, or it is the same test with more machinery.
#[test]
fn discovery_finds_the_crates_a_list_missed() {
    let checkout = Checkout::read();
    for fork in FORKS {
        let name = fork.dependency;
        let found = detached_crates_linking(&checkout, fork);
        for crate_dir in fork.detached {
            assert!(
                found.iter().any(|entry| entry == crate_dir),
                "{crate_dir} declares its own workspace and links {name}, so discovery \
                 must find it; it found {found:?}"
            );
        }
        for crate_dir in fork.not_linking {
            assert!(
                !found.iter().any(|entry| entry == crate_dir),
                "{crate_dir} compiles no {name}, so discovery must not find it linking it"
            );
        }
    }
}
