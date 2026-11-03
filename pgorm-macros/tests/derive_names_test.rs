//! The derive catalogue read off the source: the names `proc_macro_derive`
//! declares are exactly the ones the rule lists, and no `use` anywhere in the
//! workspace's crates imports or re-exports a `Derive`-prefixed name, so none
//! survives as an alias.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use syn::{Item, Meta, UseTree};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("pgorm-macros sits in the workspace")
        .to_path_buf()
}

fn parse(path: &Path) -> syn::File {
    let source = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    syn::parse_file(&source).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The first name in each `#[proc_macro_derive(Name, ..)]` on a function.
fn derives_declared(path: &Path) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for item in parse(path).items {
        let Item::Fn(function) = item else { continue };
        for attr in &function.attrs {
            if !attr.path().is_ident("proc_macro_derive") {
                continue;
            }
            let Meta::List(list) = &attr.meta else {
                continue;
            };
            let name = list
                .tokens
                .clone()
                .into_iter()
                .next()
                .expect("a derive names itself first")
                .to_string();
            names.insert(name);
        }
    }
    names
}

fn is_old_name(name: &str) -> bool {
    let derive_prefixed = name
        .strip_prefix("Derive")
        .and_then(|rest| rest.chars().next())
        .is_some_and(char::is_uppercase);
    // syn's `DeriveInput` is the input type of every derive rather than a
    // derive.
    (derive_prefixed && !name.starts_with("DeriveInput")) || name == "FromJsonQueryResult"
}

fn old_names_in_tree(tree: &UseTree, found: &mut Vec<String>) {
    match tree {
        UseTree::Path(path) => old_names_in_tree(&path.tree, found),
        UseTree::Name(name) if is_old_name(&name.ident.to_string()) => {
            found.push(name.ident.to_string());
        }
        UseTree::Rename(rename) => {
            for ident in [&rename.ident, &rename.rename] {
                if is_old_name(&ident.to_string()) {
                    found.push(format!("{} as {}", rename.ident, rename.rename));
                }
            }
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                old_names_in_tree(tree, found);
            }
        }
        _ => {}
    }
}

fn old_names_in_items(items: &[Item], found: &mut Vec<String>) {
    for item in items {
        match item {
            Item::Use(item) => old_names_in_tree(&item.tree, found),
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    old_names_in_items(items, found);
                }
            }
            _ => {}
        }
    }
}

fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

// [spec:pgorm:req:macros.derive.names+1/test]    the declared derives are the rule's list, none `Derive`-prefixed
#[test]
fn derives_are_named_for_what_they_make() {
    let root = workspace();
    let declared = derives_declared(&root.join("pgorm-macros/src/lib.rs"));
    let expected: BTreeSet<String> = [
        "ActiveEnum",
        "Column",
        "CreatedRange",
        "Disjoint",
        "Entity",
        "EntityTrait",
        "FromQueryResult",
        "IntoActiveModel",
        "JsonValueType",
        "MigrationName",
        "Model",
        "PartialModel",
        "Relation",
        "SqlName",
        "StaticName",
        "ValueType",
    ]
    .map(String::from)
    .into();
    assert_eq!(declared, expected);

    let query = derives_declared(&root.join("pgorm-query/pgorm-query-derive/src/lib.rs"));
    assert_eq!(query, ["SqlName", "StaticName"].map(String::from).into());
}

// [spec:pgorm:req:macros.derive.names+1/test]    no crate imports or re-exports an old name, under any alias
#[test]
fn no_old_derive_name_is_imported_or_aliased() {
    let root = workspace();
    let mut files = Vec::new();
    for dir in [
        "src",
        "pgorm-macros/src",
        "pgorm-migration/src",
        "pgorm-codegen/src",
        "pgorm-query/src",
        "pgorm-query/pgorm-query-attr/src",
        "pgorm-query/pgorm-query-derive/src",
        "pgorm-query/pgorm-query-derive-internals/src",
    ] {
        rust_files(&root.join(dir), &mut files);
    }

    let mut found = Vec::new();
    for file in &files {
        let mut names = Vec::new();
        old_names_in_items(&parse(file).items, &mut names);
        found.extend(
            names
                .into_iter()
                .map(|name| format!("{}: {name}", file.display())),
        );
    }
    assert_eq!(found, Vec::<String>::new());
}
