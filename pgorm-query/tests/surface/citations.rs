//! The spec cites a rule by its id alone, and every citation names the rule's
//! own verb.
//!
//! A rule's marker, `> [spec:pgorm:req:a.b+3]`, states the rule's version, and
//! code cites a rule at a version (`[spec:pgorm:req:a.b+3]`, or `+3/test` from
//! a test) so that a bump finds every site written against the old text. Prose
//! has no use for the number: a pin there goes stale at the cited rule's next
//! bump, and correcting it edits the citing rule's body, a change nspec's
//! convention answers with a bump of the citing rule too, so a stale number
//! tends to stay. Rule text and the prose around it therefore cite
//! `[spec:pgorm:req:a.b]`, or `a.b` in backticks, and these tests refuse a
//! pinned citation anywhere under `docs/spec` except a marker.
//!
//! The verb is part of a rule's identity, declared by its marker: the rule
//! `> [spec:pgorm:req:a.b+3]` opens is `req:a.b`, and `[spec:pgorm:sem:a.b]`
//! cites no rule the spec has. nspec matches a citation on its id alone and
//! counts that one as the rule's anyway, so nothing else notices the verb is
//! wrong, and these tests hold every bracketed citation in the checkout (code,
//! tests, plans, decisions, the spec) to the verb its rule declares.

use std::{collections::BTreeMap, fs, path::Path, process::Command};

use crate::spec::{
    bracketed_citations, checkout_root, citations, declared_verbs, defined_id, read_spec_file,
    spec_dir, spec_files,
};

/// Each pinned citation in `line`, as `a.b+3`.
fn pins(line: &str) -> Vec<String> {
    citations(line)
        .filter_map(|cited| Some(format!("{}+{}", cited.id, cited.version?)))
        .collect()
}

/// A pin in prose would go stale at the cited rule's next bump. The message
/// names each one so that dropping its `+N` is the whole fix.
#[test]
fn the_spec_cites_no_rule_at_a_version() {
    let dir = spec_dir();
    let mut markers = 0;
    let mut pinned = Vec::new();
    for file in spec_files() {
        let name = file
            .strip_prefix(&dir)
            .unwrap_or(&file)
            .display()
            .to_string();
        for (index, line) in read_spec_file(&file).lines().enumerate() {
            if defined_id(line).is_some() {
                markers += 1;
                continue;
            }
            for pin in pins(line) {
                pinned.push(format!("docs/spec/{name}:{}: {pin}", index + 1));
            }
        }
    }
    assert!(markers > 0, "docs/spec defines no rule");
    assert!(
        pinned.is_empty(),
        "rule text cites these rules at a version; cite each by its id alone, without the +N:\n{}",
        pinned.join("\n")
    );
}

/// The scan is only as good as what it tells apart, so its cases are held
/// here rather than inferred from a spec that may happen to have none.
#[test]
fn a_pin_is_told_from_other_plus_signs() {
    assert_eq!(
        defined_id("> [spec:pgorm:req:sql.ddl.view+1]"),
        Some("sql.ddl.view")
    );
    assert_eq!(
        defined_id("> [spec:pgorm:def:sql.value.interval]"),
        Some("sql.value.interval")
    );
    assert_eq!(
        defined_id("> [spec:pgorm:req:query.loader.table-ref-limitation+3] rather than SQL"),
        None
    );
    assert_eq!(
        defined_id("> [spec:pgorm:req:a.b] and [spec:pgorm:req:c.d]"),
        None
    );
    assert_eq!(defined_id("> see [spec:pgorm:req:sql.ddl.view+1]"), None);
    assert_eq!(defined_id("> [spec:pgorm:req:sql.ddl.view+1/test]"), None);

    assert_eq!(
        pins("> (`[spec:pgorm:req:sql.ddl.create-table+16]`) and `exec.cursor.keyset+6`."),
        ["sql.ddl.create-table+16", "exec.cursor.keyset+6"]
    );
    assert_eq!(
        pins("> [spec:pgorm:req:query.loader.table-ref-limitation+3] rather than"),
        ["query.loader.table-ref-limitation+3"]
    );
    assert_eq!(
        pins("> as bare text, sql.scope+11, it reads"),
        ["sql.scope+11"]
    );
    assert_eq!(pins("> as it ends, sql.scope+17."), ["sql.scope+17"]);
    assert_eq!(
        pins("> `[spec:pgorm:req:sql.ddl.view+1/test]`"),
        ["sql.ddl.view+1"]
    );

    assert!(pins("> (`[spec:pgorm:req:sql.ddl.view]`), `sql.ast.copy`").is_empty());
    assert!(pins("> `1e300` as `1e+300`, always at `+00:00`").is_empty());
    assert!(pins("> `+1 months -2 days +3000004 microseconds`").is_empty());
    assert!(pins("> `Value.x+1`, `a_b.c+2`, `N+1`, `x + 1`").is_empty());
}

/// Every file in the checkout that git does not ignore, tracked or not yet
/// added, as a path from its root: a new file is held to its verbs before it
/// is committed, and a build output is never read.
fn checkout_files(root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args(["ls-files", "-co", "--exclude-standard", "-z"])
        .current_dir(root)
        .output()
        .unwrap_or_else(|error| panic!("running git ls-files in {}: {error}", root.display()));
    assert!(
        output.status.success(),
        "git ls-files failed in {}: {}",
        root.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git ls-files names every path in UTF-8")
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A plan log entry's text, the one field the plan's schema calls `text`. The
/// log is append-only history, so a citation an entry once wrote stays as it
/// was written.
fn is_plan_log_text(path: &str, line: &str) -> bool {
    path == "plan/main.styx" && line.trim_start().starts_with("text ")
}

/// Each citation in `line` under a verb its rule is not declared under, as
/// `sem:a.b is declared req`. A citation of an id the spec does not declare
/// has no verb to differ from, and passes.
fn misnamed<'a>(
    declared: &'a BTreeMap<String, String>,
    line: &'a str,
) -> impl Iterator<Item = String> + 'a {
    bracketed_citations(line).filter_map(|cited| {
        let verb = cited.verb?;
        let declared = declared.get(cited.id)?;
        (verb != declared).then(|| format!("{verb}:{} is declared {declared}", cited.id))
    })
}

/// The message names each citation so that writing the declared verb is the
/// whole fix.
#[test]
fn every_citation_names_its_rules_verb() {
    let declared = declared_verbs();
    assert!(!declared.is_empty(), "docs/spec defines no rule");
    let root = checkout_root();
    let mut wrong = Vec::new();
    for path in checkout_files(&root) {
        // A binary file holds no citation, and one deleted since it was
        // added is listed but gone.
        let Ok(text) = fs::read_to_string(root.join(&path)) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if !is_plan_log_text(&path, line) {
                let site = |found| format!("{path}:{}: {found}", index + 1);
                wrong.extend(misnamed(&declared, line).map(site));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "these citations name a verb their rule is not declared under; write the verb the rule's \
         marker in docs/spec declares:\n{}",
        wrong.join("\n")
    );
}

/// The check's cases, against a declaration of their own: a fixture here
/// that cited a rule the spec declares would be read by the check itself.
#[test]
fn a_verb_is_read_wherever_a_citation_stands() {
    let declared = BTreeMap::from([
        ("a.b".to_owned(), "req".to_owned()),
        ("c.d".to_owned(), "sem".to_owned()),
    ]);
    let wrong = |line| misnamed(&declared, line).collect::<Vec<_>>();

    assert_eq!(
        wrong("// [spec:pgorm:sem:a.b+2]"),
        ["sem:a.b is declared req"]
    );
    assert_eq!(
        wrong("rules ([spec:pgorm:req:a.b] [spec:pgorm:req:c.d])"),
        ["req:c.d is declared sem"]
    );
    assert_eq!(
        wrong("notes.md:[spec:pgorm:def:a.b+1/test]"),
        ["def:a.b is declared req"]
    );
    assert_eq!(wrong("[[spec:pgorm:def:c.d]]"), ["def:c.d is declared sem"]);
    assert_eq!(
        wrong("`[spec:pgorm:sem:a.b]`, \"[spec:pgorm:req:c.d]\""),
        ["sem:a.b is declared req", "req:c.d is declared sem"]
    );

    assert!(wrong("// [spec:pgorm:req:a.b+2/test] and `[spec:pgorm:sem:c.d]`").is_empty());
    assert!(wrong("[spec:pgorm:def:e.f], sem:a.b, `a.b`").is_empty());
    assert!(wrong("[spec:pgorm:<verb>:<rule-id>(+N)?(/facet)?]").is_empty());

    assert!(is_plan_log_text(
        "plan/main.styx",
        "                text \"[spec:pgorm:sem:a.b]\""
    ));
    assert!(!is_plan_log_text(
        "plan/main.styx",
        "                [spec:pgorm:sem:a.b]"
    ));
    assert!(!is_plan_log_text(
        "docs/notes.md",
        "text \"[spec:pgorm:sem:a.b]\""
    ));
}
