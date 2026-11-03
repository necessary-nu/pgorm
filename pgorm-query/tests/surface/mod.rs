//! `sql.surface`, held where nextest runs it.
//!
//! The rule is about the text of `pgorm-query/src/lib.rs` — which modules are
//! public, what is exported and how — and about the register in
//! `docs/spec/sql-ast.md` that records each export added since the rule was
//! first written, so these tests read both: the source as a syntax tree, the
//! register as the prose it is. That an internal shape cannot be named from
//! outside the crate is the compiler's to say, so `compile-fail/` asks it.
//!
//! The spec's reader lives here, so `citations` holds the whole spec to how
//! its prose cites a rule as well, and every citation in the checkout to the
//! verb its rule is declared under.

mod baseline;
mod citations;
mod source;
mod spec;

use std::collections::{BTreeMap, BTreeSet};

use source::{Source, crate_sources};
use spec::{Register, Rule, code_spans, defined_rule_ids, is_item_shaped};

/// The phrase that opens `sql.surface`'s register of additions.
const REGISTER_OPENS: &str = "The list grows only by";

/// The phrase that opens the rule's departures, which end the register: a
/// name after it records an item that left the surface, not one on it.
const DEPARTURES_OPEN: &str = "An item leaves the list";

/// The sentence of the rule's residue paragraph that makes the list a MUST.
const RESIDUE_SENTENCE: &str = "named individually";

/// The modules the rule keeps public, each because a path through it is the
/// spelling a caller writes.
const PUBLIC_MODULES: [&str; 3] = ["error", "extension", "value"];

/// The only module public inside those three: the path to
/// `value::with_array::NotU8`, which pgorm's derives write in generated code.
const NESTED_PUBLIC_MODULES: [&str; 1] = ["value::with_array"];

/// The `#[doc(hidden)]` exports the rule names: reachable, because narrowing
/// them would only make them dead code, and kept out of the rendered API.
const RESIDUE: [&str; 3] = ["SelectDistinct", "Token", "Tokenizer"];

/// The phrase that ends the rule's open enums and opens its exhaustive ones,
/// in the one paragraph that classifies every exported enum.
const EXHAUSTIVE_OPENS: &str = "An enum stays exhaustive";

/// The phrase that ends the rule's exhaustive enums.
const EXHAUSTIVE_ENDS: &str = "A match on an open enum";

fn surface_rule() -> Rule {
    Rule::read("sql-ast.md", "sql.surface")
}

fn set<'a>(names: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    names.into_iter().map(str::to_owned).collect()
}

/// The register's span of the rule's text: from the phrase that opens it to
/// the one that opens the departures.
fn register_span(text: &str) -> &str {
    let missing = |phrase: &str| {
        format!(
            "sql.surface (docs/spec/sql-ast.md) no longer contains {phrase:?}, which this test \
             reads the register's bounds from: update REGISTER_OPENS and DEPARTURES_OPEN in \
             pgorm-query/tests/surface/mod.rs to the rule's wording"
        )
    };
    let start = text
        .find(REGISTER_OPENS)
        .unwrap_or_else(|| panic!("{}", missing(REGISTER_OPENS)));
    let end = text[start..]
        .find(DEPARTURES_OPEN)
        .unwrap_or_else(|| panic!("{}", missing(DEPARTURES_OPEN)));
    &text[start..start + end]
}

// [spec:pgorm:req:sql.surface+38/test]    exactly `error`, `extension` and `value` are public,
// `tests_cfg` hidden behind its feature, no module inside them public but
// `value::with_array`, `error` exporting what the rule names, and every item
// `value` exports exported at the root as well
#[test]
fn exactly_three_modules_are_public() {
    let lib = Source::read("src/lib.rs");
    let modules = lib.public_modules();

    let visible: BTreeSet<String> = modules
        .iter()
        .filter(|module| !module.hidden)
        .map(|module| module.path.clone())
        .collect();
    assert_eq!(
        visible,
        set(PUBLIC_MODULES),
        "sql.surface keeps exactly three modules public; an item reaches the API through \
         lib.rs's `pub use` list, never by sitting in a module a caller can name"
    );

    let hidden: Vec<(&str, &[String])> = modules
        .iter()
        .filter(|module| module.hidden)
        .map(|module| (module.path.as_str(), module.cfg.as_slice()))
        .collect();
    assert_eq!(
        hidden,
        [(
            "tests_cfg",
            [r#"feature = "tests-cfg""#.to_owned()].as_slice()
        )],
        "the one hidden public module is `tests_cfg`, behind its own feature"
    );

    let nested: BTreeSet<String> = PUBLIC_MODULES
        .iter()
        .flat_map(|module| {
            Source::read(&format!("src/{module}.rs"))
                .public_modules()
                .into_iter()
                .map(move |inner| format!("{module}::{}", inner.path))
        })
        .collect();
    assert_eq!(
        nested,
        set(NESTED_PUBLIC_MODULES),
        "a module inside a public one is a path callers can write, so it is private unless the \
         rule names it"
    );

    assert_eq!(
        set(Source::read("src/error.rs")
            .exported_names()
            .iter()
            .map(String::as_str)),
        set(["Error", "Result", "TemplateError"]),
        "`error` exports what sql.surface names for it"
    );

    let root = set(lib.exported_names().iter().map(String::as_str));
    let value_only: Vec<String> = Source::read("src/value.rs")
        .exported_names()
        .into_iter()
        .filter(|name| !root.contains(name))
        .collect();
    assert!(
        value_only.is_empty(),
        "`value` exports {value_only:?}, which lib.rs does not: an item is API through the root \
         list, and `value` stays public only for the `with_array` path"
    );
}

// [spec:pgorm:req:sql.surface+38/test]    every export of lib.rs and of the three public
// modules is a named item: none is a glob
#[test]
fn exports_are_named_never_globbed() {
    for file in [
        "src/lib.rs",
        "src/error.rs",
        "src/extension.rs",
        "src/value.rs",
    ] {
        let source = Source::read(file);
        let globs = source.exports().iter().filter(|export| export.glob).count();
        assert_eq!(
            globs,
            0,
            "{} exports {globs} glob(s): sql.surface names every exported item, so the list is \
             what the crate exports and a module's later additions do not join it unseen",
            source.path.display()
        );
    }
}

// [spec:pgorm:req:sql.surface+38/test]    lib.rs's `#[doc(hidden)]` exports are the residue
// the rule names, and every export the rule's residue paragraph names is one of them
#[test]
fn doc_hidden_residue_is_the_named_list() {
    let lib = Source::read("src/lib.rs");
    let exports = lib.exports();
    let hidden = exports
        .iter()
        .filter(|export| export.hidden)
        .map(|export| export.name.as_str());
    assert_eq!(
        set(hidden),
        set(RESIDUE),
        "lib.rs's `#[doc(hidden)]` exports are not the residue sql.surface names: an item hidden \
         from the documentation is a named debt, so the rule and RESIDUE change with it"
    );

    let rule = surface_rule();
    let exported = set(exports.iter().map(|export| export.name.as_str()));
    let named = code_spans(rule.paragraph_with(RESIDUE_SENTENCE))
        .filter(|span| is_item_shaped(span) && exported.contains(*span));
    assert_eq!(
        set(named),
        set(RESIDUE),
        "the exports sql.surface's residue paragraph names are not RESIDUE"
    );
}

// [spec:pgorm:req:sql.surface+38/test]    the register and the exports agree both ways: an
// export of lib.rs or `extension` outside the baseline has an entry, each entry names an
// export and ends with a rule the spec defines, and the baseline holds only what is still
// exported
#[test]
fn register_and_exports_agree_both_ways() {
    let root = set(Source::read("src/lib.rs")
        .exported_names()
        .iter()
        .map(String::as_str));
    let extension = set(Source::read("src/extension.rs")
        .exported_names()
        .iter()
        .map(String::as_str));
    let exported = |name: &str| root.contains(name) || extension.contains(name);

    let text = surface_rule().text();
    let register = Register::parse(register_span(&text), exported);
    let defined = defined_rule_ids();
    let mut problems = Vec::new();

    for (file, exports, baseline) in [
        ("src/lib.rs", &root, baseline::ROOT),
        ("src/extension.rs", &extension, baseline::EXTENSION),
    ] {
        for name in exports {
            let recorded =
                register.entries.contains_key(name) || register.without_rule.contains(name);
            if !baseline.contains(&name.as_str()) && !recorded {
                problems.push(format!(
                    "`{name}` is exported by pgorm-query/{file} but sql.surface's register has \
                     no entry for it. Record it in docs/spec/sql-ast.md, before the sentence \
                     \"{DEPARTURES_OPEN}\": the name in backticks, what it is, then the rule \
                     that specifies it in parentheses — `{name}`, <what it is> (`<rule.id>`) — \
                     and bump the rule's version."
                ));
            }
        }
        for name in baseline {
            if !exports.contains(*name) {
                problems.push(format!(
                    "`{name}` is in pgorm-query/tests/surface/baseline.rs, but pgorm-query/{file} \
                     no longer exports it: delete it from the baseline, and say in sql.surface's \
                     departures (\"{DEPARTURES_OPEN} ...\") what replaced it."
                ));
            }
        }
    }

    for name in register.entries.keys().chain(&register.without_rule) {
        if !exported(name) {
            problems.push(format!(
                "sql.surface's register names `{name}`, which neither lib.rs nor `extension` \
                 exports. An item that left the surface belongs in the departures sentence \
                 (\"{DEPARTURES_OPEN} ...\"), with what replaced it; an item that is not exported \
                 has no entry."
            ));
        }
    }
    for name in &register.without_rule {
        problems.push(format!(
            "`{name}` comes after the last rule sql.surface's register cites: an entry ends \
             with the rule that specifies it, in parentheses — (`<rule.id>`)."
        ));
    }
    for rule in register.cited_rules() {
        if !defined.contains(rule) {
            problems.push(format!(
                "sql.surface's register cites `{rule}`, which no file under docs/spec defines."
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "sql.surface's register and pgorm-query's exports disagree:\n\n{}",
        problems.join("\n\n")
    );
}

// [spec:pgorm:req:sql.surface+38/test]    every exported enum is named once, open or
// exhaustive, and is `#[non_exhaustive]` exactly when the rule names it open
#[test]
fn exported_enums_are_open_or_closed_as_named() {
    let exported: BTreeSet<String> = [
        "src/lib.rs",
        "src/error.rs",
        "src/extension.rs",
        "src/value.rs",
    ]
    .into_iter()
    .flat_map(|file| Source::read(file).exported_names())
    .collect();
    let enums: BTreeMap<String, bool> = crate_sources()
        .iter()
        .flat_map(Source::public_enums)
        .filter(|(name, _)| exported.contains(name))
        .collect();

    let rule = surface_rule();
    let paragraph = rule.paragraph_with(EXHAUSTIVE_OPENS);
    let (open, rest) = paragraph
        .split_once(EXHAUSTIVE_OPENS)
        .unwrap_or_else(|| panic!("sql.surface no longer contains {EXHAUSTIVE_OPENS:?}"));
    let (closed, _) = rest
        .split_once(EXHAUSTIVE_ENDS)
        .unwrap_or_else(|| panic!("sql.surface no longer contains {EXHAUSTIVE_ENDS:?}"));
    let named = |text: &str| -> BTreeSet<String> {
        code_spans(text)
            .filter(|span| enums.contains_key(*span))
            .map(str::to_owned)
            .collect()
    };
    let (open, closed) = (named(open), named(closed));

    let mut problems = Vec::new();
    for (name, non_exhaustive) in &enums {
        match (open.contains(name), closed.contains(name)) {
            (true, true) => problems.push(format!("`{name}` is named both open and exhaustive")),
            (false, false) => problems.push(format!(
                "`{name}` is exported but sql.surface names it neither open nor exhaustive: \
                 decide which by the rule's test and name it in that list"
            )),
            (true, false) if !non_exhaustive => problems.push(format!(
                "sql.surface names `{name}` open, but it is not `#[non_exhaustive]`"
            )),
            (false, true) if *non_exhaustive => problems.push(format!(
                "sql.surface names `{name}` exhaustive, but it is `#[non_exhaustive]`"
            )),
            _ => {}
        }
    }
    assert!(
        problems.is_empty(),
        "pgorm-query's exported enums and sql.surface's classification of them disagree:\n\n{}",
        problems.join("\n")
    );
}

// [spec:pgorm:req:sql.surface+38/test]    the compiler refuses every path to `Write`, `Mode`,
// `Oper` and the clause shapes a statement holds internally
//
// The snapshot is rustc's own wording, and the refusal it records is the
// contract: each name an `E0603` (private) or, for `Mode`, which lib.rs never
// imports, an `E0432`. A toolchain that rewords a diagnostic fails this until
// `TRYBUILD=overwrite` rewrites the snapshot, which is right only once every
// entry in it is still one of those two refusals.
#[test]
fn internal_shapes_cannot_be_named_outside() {
    trybuild::TestCases::new().compile_fail("./tests/surface/compile-fail/*.rs");
}
