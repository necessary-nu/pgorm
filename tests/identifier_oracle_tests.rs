#![allow(unused_imports, dead_code)]

//! The identifier render oracle.
//!
//! sqlmap cannot fire in identifier positions: a level-3 boundary cannot close
//! a faithfully double-quoted identifier and attach a payload, so the cases
//! whose input is a name carry few or no scheduled techniques
//! (`security/sqlmap/README.md`). This oracle judges those positions
//! structurally instead. Every public API that renders a caller-supplied name
//! into SQL text is registered (`identifier_oracle/registry*.rs`); each is
//! rendered with every name in a hostile corpus; and each statement is parsed
//! by libpg_query and compared, node for node, with the same statement
//! rendered with a benign name. A name that changed the statement's shape —
//! closed its quote, ended the statement, opened a comment, turned into a
//! keyword — shows up as a difference in the tree, whatever payload a scanner
//! would have needed to find it.
//!
//! The live leg (`identifier_oracle/live.rs`) runs the nastiest names against
//! a real server, so the parser's reading is checked against the server's, and
//! `identifier_oracle/live_capture.rs` holds the sites whose SQL only exists on
//! its way to a server — captured from tokio-postgres's statement log — to the
//! same property as the rest.

use pgorm_query::ExprTrait;
pub mod common;
mod identifier_oracle;

use std::collections::{BTreeMap, BTreeSet};

use identifier_oracle::{
    corpus::{corpus, scanner_keywords},
    oracle::{Policy, Site, Verdict, judge, judge_against, judge_nul, reference},
    pins::{PINS, pinned},
    registry::{n_, sites, sql},
};
use pgorm::pgorm_query::{Expr, Query};

/// Every site's benign rendering parses and puts the name exactly where the
/// registry says, and no two sites share an id.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[test]
fn every_site_declares_where_its_name_lands() {
    let sites = sites();
    let mut ids = BTreeSet::new();
    let failures: Vec<String> = sites
        .iter()
        .filter_map(|site| {
            if !ids.insert(site.id) {
                return Some(format!("site id `{}` is registered twice", site.id));
            }
            reference(site)
                .err()
                .map(|err| format!("site `{}` ({}): {err}", site.id, site.api))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} sites misdeclared:\n{}",
        failures.len(),
        sites.len(),
        failures.join("\n")
    );
}

/// Every hostile name at every site either round-trips as exactly the
/// declared identifiers with the statement's shape unchanged, or meets the
/// outcome its policy requires instead — a refusal, the grammar's own
/// rejection of the empty name, a listed type spelling read as its type, a
/// listed call form read as its expression. A failure
/// names the site, the name and the structural difference.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[test]
fn every_site_holds_every_hostile_name() {
    let sites = sites();
    let corpus = corpus();
    let mut failures = Vec::new();
    let mut tally: BTreeMap<&str, BTreeMap<Verdict, usize>> = BTreeMap::new();
    for site in &sites {
        let Ok(reference) = reference(site) else {
            continue;
        };
        for hostile in &corpus {
            let outcome = judge_against(site, hostile, (site.render)(&hostile.name), &reference);
            match (outcome, pinned(site.id, hostile.label)) {
                (Ok(verdict), None) => {
                    *tally
                        .entry(site.crate_name())
                        .or_default()
                        .entry(verdict)
                        .or_default() += 1;
                }
                (Err(_), Some(_)) => {}
                (Err(failure), None) => failures.push(failure),
                (Ok(verdict), Some(pin)) => failures.push(format!(
                    "site `{}`, name `{}`: pinned as defect `{}` but now passes ({verdict:?}); \
                     retire the pin",
                    site.id, hostile.label, pin.node
                )),
            }
        }
    }
    eprintln!("verdicts by crate: {tally:#?}");
    assert!(
        failures.is_empty(),
        "{} site × name failures across {} sites and {} names:\n\n{}",
        failures.len(),
        sites.len(),
        corpus.len(),
        failures.join("\n\n")
    );
}

/// The byte offsets PostgreSQL 18's parse tree records beside `location` —
/// an `IN` list's bounds, an array constructor's — move with the length of
/// every name written before them, so the comparison sets them aside as it
/// does `location`. No registered site's statement puts one after its name,
/// since each is the smallest statement holding its position, so these sites
/// exist to hold the comparison itself: every corpus name before an `IN` list
/// and before an `ARRAY` constructor reads back as the name with nothing else
/// moved.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[test]
fn offsets_after_a_name_are_set_aside() {
    let sites = [
        Site {
            id: "oracle/offset.in-list",
            api: "Expr::col(Name).is_in(..)",
            kinds: &["ColumnRef.fields[0]"],
            policy: Policy::Quoted,
            render: |n| sql(Query::select().expr(Expr::col(n_(n)).is_in([1, 2]))),
        },
        Site {
            id: "oracle/offset.array",
            api: "SelectStatement::column(Name) before an array value",
            kinds: &["ColumnRef.fields[0]"],
            policy: Policy::Quoted,
            render: |n| sql(Query::select().column(n_(n)).expr(Expr::val(vec![1, 2]))),
        },
    ];
    let corpus = corpus();
    let mut failures = Vec::new();
    for site in &sites {
        let reference = match reference(site) {
            Ok(reference) => reference,
            Err(err) => panic!("site `{}` ({}): {err}", site.id, site.api),
        };
        for hostile in &corpus {
            if let Err(failure) =
                judge_against(site, hostile, (site.render)(&hostile.name), &reference)
            {
                failures.push(failure);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} site × name failures:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// Every keyword the linked scanner knows — a few hundred, where the corpus
/// holds ten — at every site under `TypeName`'s part policy either
/// round-trips as a name, or is one of the grammar's own forms that
/// `sql.types.type-name` lists per keyword and the oracle accepts per keyword
/// (`TYPE_SPELLINGS` at a type, `CALL_FORMS` at a function). So the policy is
/// held for the whole keyword list, not only the words the corpus happens to
/// hold, and a keyword whose bare spelling means something else can only
/// render quoted.
// [spec:pgorm:def:sql.types.type-name+8/test]
// [spec:pgorm:req:security.ident-oracle+17/test]
#[test]
fn type_name_sites_hold_every_keyword() {
    let keywords = scanner_keywords();
    assert!(
        keywords.len() > 400,
        "the scanner knows {} keywords",
        keywords.len()
    );
    let mut failures = Vec::new();
    let mut tally: BTreeMap<&str, BTreeMap<Verdict, usize>> = BTreeMap::new();
    for site in sites()
        .iter()
        .filter(|site| matches!(site.policy, Policy::TypePart | Policy::FunctionName))
    {
        let reference = reference(site).expect("every site's benign rendering is checked above");
        for keyword in &keywords {
            match judge_against(site, keyword, (site.render)(&keyword.name), &reference) {
                Ok(verdict) => {
                    *tally
                        .entry(site.id)
                        .or_default()
                        .entry(verdict)
                        .or_default() += 1
                }
                Err(failure) => failures.push(failure),
            }
        }
    }
    eprintln!("keyword verdicts by site: {tally:#?}");
    assert!(
        failures.is_empty(),
        "{} site × keyword failures:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// A NUL-bearing name is refused by the API, or reaches the statement text
/// where the protocol encoder refuses it, or is escaped into a literal the
/// grammar refuses — whichever the site's policy declares, and never
/// anything that parses.
// [spec:pgorm:req:security.ident-oracle.nul+2/test]
#[test]
fn every_site_keeps_nul_out_of_the_server() {
    let sites = sites();
    let mut by_behaviour: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let failures: Vec<String> = sites
        .iter()
        .filter_map(|site| match judge_nul(site) {
            Ok(behaviour) => {
                by_behaviour
                    .entry(format!("{behaviour:?}"))
                    .or_default()
                    .push(site.id);
                None
            }
            Err(failure) => Some(failure),
        })
        .collect();
    eprintln!("NUL behaviour by site: {by_behaviour:#?}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Each pinned defect still reproduces exactly as filed. A pin fails the
/// moment its site × name pair starts passing, so a fix cannot land without
/// the pin being retired, and the defect is reported on every run until then.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[test]
fn pinned_identifier_defects_still_reproduce() {
    let sites = sites();
    let corpus = corpus();
    for pin in PINS {
        for label in pin.labels {
            let site = sites
                .iter()
                .find(|site| site.id == pin.site)
                .unwrap_or_else(|| panic!("pin names unregistered site `{}`", pin.site));
            let hostile = corpus
                .iter()
                .find(|hostile| hostile.label == *label)
                .unwrap_or_else(|| panic!("pin names unknown corpus entry `{label}`"));
            match judge(site, hostile) {
                Err(failure) => eprintln!("PINNED DEFECT {}: {failure}", pin.node),
                Ok(verdict) => panic!(
                    "pin `{}` for site `{}`, name `{label}` is stale: it now passes ({verdict:?})",
                    pin.node, pin.site
                ),
            }
        }
    }
}
