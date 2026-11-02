use pgorm_sqlmap_adapter::harness::{
    self, Options, Settings, process,
    result::{self, ScanResult},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const TARGET: &str = "http://127.0.0.1:56392/case/protected/select?input=alice";
const LOG: &str = "GET parameter 'input' does not seem to be injectable\n[*] ending @ 12:00:00";

fn clean_report() -> Value {
    json!({"success":true,"meta":{"url":TARGET.split('?').next()},"data":[],"error":[result::CLEAN_MESSAGE]})
}
fn clean() -> ScanResult {
    result::interpret(&clean_report(), LOG, Some(0), TARGET, "input", 25)
}
fn finding(technique: &str, parameter: &str, place: &str) -> Value {
    json!({"parameter":parameter,"place":place,"data":[{"technique":technique,"payload":"input=alice' AND 1=1 --"}]})
}
fn detection() -> ScanResult {
    let mut report = clean_report();
    report["data"] = json!([{"type_name":"TECHNIQUES","value":[finding("boolean-based blind", "input", "GET")]}]);
    result::interpret(&report, LOG, Some(0), TARGET, "input", 25)
}

// [spec:pgorm:req:security.sqlmap.runner-tests]
// [spec:pgorm:req:security.sqlmap.runner-tests/test]
// [spec:pgorm:req:security.sqlmap.execution/test]
#[test]
fn sqlmap_urls_omit_query_but_match_endpoints() {
    for (raw, log) in [
        (
            include_str!("fixtures/control.json"),
            include_str!("fixtures/control.log"),
        ),
        (
            include_str!("fixtures/protected.json"),
            include_str!("fixtures/protected.log"),
        ),
    ] {
        let report: Value = serde_json::from_str(raw).unwrap();
        let target = format!("{}?input=alice", report["meta"]["url"].as_str().unwrap());
        let complete = result::interpret(&report, log, Some(0), &target, "input", 25);
        assert!(complete.complete, "{}", complete.reason);
        for (field, stale) in [
            ("scheme", target.replace("http:", "https:")),
            ("hostname", target.replace("127.0.0.1", "localhost")),
            ("port", target.replace(":56392", ":56393")),
            ("path", target.replace("/select", "/delete")),
        ] {
            let failed = result::interpret(&report, log, Some(0), &stale, "input", 25);
            assert!(!failed.complete);
            assert!(failed.reason.contains(field), "{}", failed.reason);
            assert_eq!(failed.findings, complete.findings);
        }
        let inactive = result::interpret(&report, log, Some(0), &target, "input", 0);
        assert!(!inactive.complete);
        assert!(inactive.reason.contains("inactive route"));
        assert_eq!(inactive.findings, complete.findings);
        let crashed = result::interpret(&report, log, Some(1), &target, "input", 25);
        assert!(crashed.reason.contains("exited 1"));
        assert_eq!(crashed.findings, complete.findings);
    }
}

#[test]
fn zero_exit_without_valid_output_is_incomplete() {
    for report in [
        Value::Null,
        json!({}),
        json!([]),
        json!({"success":false}),
        json!({"success":true}),
        json!({"success":true,"data":{},"error":[]}),
    ] {
        assert!(!result::interpret(&report, LOG, Some(0), TARGET, "input", 25).complete);
    }
}

#[test]
fn malformed_findings_and_wrong_injection_points_are_retained() {
    for finding in [
        Value::Null,
        json!("bad"),
        finding("boolean-based blind", "other", "GET"),
        finding("boolean-based blind", "input", "Cookie"),
        finding("unknown", "input", "GET"),
    ] {
        let mut report = clean_report();
        report["data"] = json!([{"type_name":"TECHNIQUES","value":[finding.clone()]}]);
        let result = result::interpret(&report, LOG, Some(0), TARGET, "input", 25);
        assert!(!result.complete);
        assert!(result.reason.contains("injection parameter"));
        assert_eq!(result.findings, vec![finding]);
    }
}

#[test]
fn malformed_metadata_never_panics_or_completes() {
    for meta in [
        Value::Null,
        json!([]),
        json!(5),
        json!({"url":[]}),
        json!({"url":"http://127.0.0.1:invalid/"}),
        json!({"url":"not a url"}),
    ] {
        let mut report = clean_report();
        report["meta"] = meta;
        let result = result::interpret(&report, LOG, Some(0), TARGET, "input", 25);
        assert!(!result.complete);
        assert!(result.reason.contains("URL"));
    }
}

#[test]
fn crashes_cancel_and_timeouts_are_incomplete() {
    for code in [Some(1), Some(-9), Some(-15), None] {
        assert!(!result::interpret(&clean_report(), LOG, code, TARGET, "input", 25).complete);
    }
}

#[test]
fn missing_terminal_evidence_is_incomplete() {
    for log in [
        "",
        "[*] ending @",
        "parameter 'input' does not seem to be injectable",
    ] {
        assert!(!result::interpret(&clean_report(), log, Some(0), TARGET, "input", 25).complete);
    }
}

#[test]
fn transport_failures_and_skipped_parameters_are_incomplete() {
    for message in [
        "connection timed out",
        "unable to connect",
        "connection reset",
        "user aborted",
        "skipping parameter",
    ] {
        let result = result::interpret(
            &clean_report(),
            &format!("{LOG}{message}"),
            Some(0),
            TARGET,
            "input",
            25,
        );
        assert!(!result.complete);
        assert!(result.reason.contains("transport"));
    }
}

#[test]
fn scanner_errors_are_not_clean_negatives() {
    let mut report = clean_report();
    report["error"] = json!(["unexpected internal exception"]);
    let result = result::interpret(&report, LOG, Some(0), TARGET, "input", 25);
    assert!(!result.complete);
    assert_eq!(result.errors, vec![json!("unexpected internal exception")]);
}

// [spec:pgorm:req:security.sqlmap.outcomes/test]
// [spec:pgorm:req:security.sqlmap.controls/test]
#[test]
fn verdict_requires_detected_control_complete_protected_and_baselines() {
    assert_eq!(
        result::verdict(&detection(), &clean(), "B", true, true),
        "pass"
    );
    assert_eq!(
        result::verdict(&clean(), &clean(), "B", true, true),
        "invalid-control"
    );
    assert_eq!(
        result::verdict(&detection(), &clean(), "E", true, true),
        "invalid-control"
    );
    let mut incomplete = clean();
    incomplete.complete = false;
    assert_eq!(
        result::verdict(&detection(), &clean(), "B", false, true),
        "incomplete"
    );
    assert_eq!(
        result::verdict(&incomplete, &clean(), "B", true, true),
        "incomplete"
    );
    assert_eq!(
        result::verdict(&detection(), &incomplete, "B", true, true),
        "incomplete"
    );
}

#[test]
fn protected_findings_and_invariant_violations_always_fail() {
    let mut incomplete = clean();
    incomplete.complete = false;
    for control in [clean(), detection(), incomplete.clone()] {
        assert_eq!(
            result::verdict(&control, &detection(), "B", false, true),
            "vulnerable"
        );
        assert_eq!(
            result::verdict(&control, &incomplete, "B", true, false),
            "vulnerable"
        );
    }
}

// [spec:pgorm:req:security.sqlmap.verdict/test]
#[test]
fn empty_extra_skipped_or_failed_cleanup_cannot_pass() {
    let pass = BTreeMap::from([("a".into(), json!({"outcome":"pass"}))]);
    assert!(result::aggregate(&["a".into()], &pass, &[], &[]));
    assert!(!result::aggregate(&[], &BTreeMap::new(), &[], &[]));
    assert!(!result::aggregate(
        &["a".into()],
        &BTreeMap::new(),
        &[],
        &[]
    ));
    assert!(!result::aggregate(
        &["a".into(), "b".into()],
        &pass,
        &[],
        &[]
    ));
    assert!(!result::aggregate(
        &["a".into(), "a".into()],
        &pass,
        &[],
        &[]
    ));
    assert!(!result::aggregate(&[], &pass, &[], &[]));
    assert!(!result::aggregate(
        &["a".into()],
        &pass,
        &["container removal failed".into()],
        &[]
    ));
    assert!(!result::aggregate(
        &["a".into()],
        &pass,
        &[],
        &["a-Q".into()]
    ));
    for outcome in [
        "incomplete",
        "invalid-control",
        "vulnerable",
        "skipped",
        "inapplicable",
    ] {
        assert!(!result::aggregate(
            &["a".into()],
            &BTreeMap::from([("a".into(), json!({"outcome":outcome}))]),
            &[],
            &[]
        ));
    }
}

// [spec:pgorm:req:security.sqlmap.profiles+4/test]
// [spec:pgorm:req:security.sqlmap.matrix/test]
#[test]
fn profiles_keep_234_full_and_six_smoke() {
    let manifest: Value = serde_json::from_str(include_str!("../../cases.json")).unwrap();
    let profiles: Value = serde_json::from_str(include_str!("../../profiles.json")).unwrap();
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 35);
    let full = result::inventory(&manifest, &profiles["full"], &[]).unwrap();
    assert_eq!(full.work.len() * 2, 234);
    assert_eq!(full.exempt.len(), 93);
    assert_eq!(
        result::inventory(&manifest, &profiles["smoke"], &[])
            .unwrap()
            .work
            .len()
            * 2,
        6
    );
    // Each smoke control must fire at smoke's own level 2 / risk 1: `parameters` needs a level-3
    // boundary and can never pass here, while `enum` escapes a double-quoted identifier at level 2.
    assert_eq!(
        profiles["smoke"]["cases"],
        json!(["select", "pipeline-literal", "enum"])
    );
    assert!(result::inventory(&manifest, &profiles["smoke"], &["insert".into()]).is_err());
    assert!(
        result::inventory(
            &manifest,
            &profiles["full"],
            &["select".into(), "select".into()]
        )
        .is_err()
    );
    let mut profile = profiles["full"].clone();
    profile["techniques"] = json!([]);
    assert!(result::inventory(&manifest, &profile, &[]).is_err());
    profile["cases"] = json!(["unknown"]);
    assert!(result::inventory(&manifest, &profile, &[]).is_err());
}

// [spec:pgorm:req:security.sqlmap.profiles+4/test]
#[test]
fn identifiers_profile_schedules_what_level_five_reaches() {
    let manifest: Value = serde_json::from_str(include_str!("../../cases.json")).unwrap();
    let profiles: Value = serde_json::from_str(include_str!("../../profiles.json")).unwrap();
    let identifiers = &profiles["identifiers"];
    assert_eq!(
        (identifiers["level"].as_u64(), identifiers["risk"].as_u64()),
        (Some(5), Some(3))
    );
    Settings::from_profile(identifiers).unwrap();
    let ids = [
        "pipeline-projection",
        "schema",
        "column",
        "group",
        "function",
        "stored-identifier",
        "graph-alias",
        "table",
        "alias",
        "order",
        "enum-ddl",
    ];
    assert_eq!(identifiers["cases"], json!(ids));
    let inventory = result::inventory(&manifest, identifiers, &[]).unwrap();
    // Measured twice at level 5 / risk 3: 28 controls fire, 38 do not (acceptance/2026-10-05-identifiers.md).
    assert_eq!(inventory.work.len(), 28);
    assert_eq!(inventory.exempt.len(), 38);
    let scheduled = |inventory: &result::Inventory| -> Vec<String> {
        let mut pairs: Vec<_> = inventory
            .work
            .iter()
            .map(|(c, t)| format!("{}-{t}", c["id"].as_str().unwrap()))
            .collect();
        pairs.sort();
        pairs
    };
    let mut expected: Vec<String> = [
        "pipeline-projection",
        "column",
        "group",
        "stored-identifier",
    ]
    .iter()
    .flat_map(|c| ["E", "T"].map(|t| format!("{c}-{t}")))
    .chain(
        ["graph-alias", "table", "alias"]
            .iter()
            .flat_map(|c| ["B", "E", "U", "S", "T"].map(|t| format!("{c}-{t}"))),
    )
    .chain(["order-B", "order-E", "order-S", "order-T", "enum-ddl-S"].map(str::to_owned))
    .collect();
    expected.sort();
    assert_eq!(scheduled(&inventory), expected);
    // The same declarations hold the full profile at level 3 to the eight pairs that fire there.
    let full = result::inventory(&manifest, &profiles["full"], &ids.map(str::to_owned)).unwrap();
    assert_eq!(full.work.len(), 8);
    let gated = full
        .exempt
        .iter()
        .filter(|(_, _, e)| e["evidence"]["ceiling"] == 5)
        .count();
    assert_eq!(gated, 20);
}

// [spec:pgorm:req:security.sqlmap.profiles+4/test]
#[test]
fn ceilings_exempt_below_and_schedule_at_their_level() {
    let mut gated = evidence();
    gated["ceiling"] = json!(5);
    let manifest = json!({"cases":[exempt_case(json!({"Q": exempt_entry(gated.clone())}))]});
    for (level, work, exempt) in [(3, &["B"][..], 1), (4, &["B"], 1), (5, &["B", "Q"], 0)] {
        let profile = json!({"cases":["a"],"techniques":["B","Q"],"level":level});
        let inventory = result::inventory(&manifest, &profile, &[]).unwrap();
        let scheduled: Vec<_> = inventory.work.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(scheduled, work, "level {level}");
        assert_eq!(inventory.exempt.len(), exempt, "level {level}");
    }
    assert!(result::exemptions(&exempt_case(json!({"Q": exempt_entry(gated)}))).is_ok());
    for bad in [
        json!(1),
        json!(6),
        json!(0),
        json!("5"),
        json!(4.5),
        Value::Null,
    ] {
        let mut broken = evidence();
        broken["ceiling"] = bad.clone();
        assert!(
            result::exemptions(&exempt_case(json!({"Q": exempt_entry(broken)}))).is_err(),
            "ceiling {bad}"
        );
    }
    // A profile that does not say its level cannot decide what a ceiling excuses.
    let profile = json!({"cases":["a"],"techniques":["B","Q"]});
    assert!(result::inventory(&json!({"cases":[exempt_case(json!({}))]}), &profile, &[]).is_err());
}

fn evidence() -> Value {
    json!({"kind":"no-boundary","payload":"inline_query.xml","where":[3],"clause":[1,2,3,8],
           "boundary":"the sole where=3 boundary carries an empty prefix and an empty suffix","contexts":"4"})
}

fn exempt_case(inapplicable: Value) -> Value {
    json!({"id":"a","techniques":["B","Q"],"inapplicable":inapplicable})
}

fn exempt_entry(evidence: Value) -> Value {
    json!({"reason":"REPLACE-only tests cannot escape the app's quoting at this injection point.","evidence":evidence})
}

// [spec:pgorm:req:security.sqlmap.profiles+4/test]
#[test]
fn exemptions_without_reason_or_evidence_are_refused() {
    assert!(result::exemptions(&exempt_case(json!({"Q": exempt_entry(evidence())}))).is_ok());
    for entry in [
        json!({}),
        json!("structurally impossible"),
        json!({"reason":"n/a","evidence":evidence()}),
        json!({"reason":"REPLACE-only tests cannot escape the app's quoting here."}),
        json!({"evidence":evidence()}),
    ] {
        assert!(
            result::exemptions(&exempt_case(json!({"Q": entry}))).is_err(),
            "{entry}"
        );
    }
    for (field, bad) in [
        ("kind", json!("unproven")),
        ("payload", json!("boolean_blind.xml")),
        ("where", json!([])),
        ("where", json!([4])),
        ("where", json!("3")),
        ("clause", json!([10])),
        ("clause", json!([])),
        ("boundary", json!("none")),
        ("contexts", json!("§4")),
        ("contexts", json!("0")),
    ] {
        let mut broken = evidence();
        broken[field] = bad.clone();
        assert!(
            result::exemptions(&exempt_case(json!({"Q": exempt_entry(broken)}))).is_err(),
            "{field}={bad}"
        );
    }
    // A missing or surplus evidence field must not be silently tolerated.
    let mut short = evidence();
    short.as_object_mut().unwrap().remove("boundary");
    assert!(result::exemptions(&exempt_case(json!({"Q": exempt_entry(short)}))).is_err());
    let mut wide = evidence();
    wide["note"] = json!("extra");
    assert!(result::exemptions(&exempt_case(json!({"Q": exempt_entry(wide)}))).is_err());
}

// [spec:pgorm:req:security.sqlmap.profiles+4/test]
#[test]
fn exemptions_naming_undeclared_techniques_are_refused() {
    for technique in ["U", "Z", "q"] {
        let mut cited = evidence();
        cited["payload"] = json!(
            result::PAYLOADS
                .iter()
                .find(|(t, _)| *t == technique)
                .map_or("union_query.xml", |(_, p)| p)
        );
        let case = exempt_case(json!({technique: exempt_entry(cited)}));
        assert!(result::exemptions(&case).is_err(), "{technique}");
    }
    assert!(result::exemptions(&json!({"id":"a","techniques":["Q"],"inapplicable":[]})).is_err());
}

// [spec:pgorm:req:security.sqlmap.profiles+4/test]
// [spec:pgorm:req:security.sqlmap.verdict/test]
#[test]
fn exempted_pairs_leave_scheduled_work_and_never_pass() {
    let manifest = json!({"cases":[exempt_case(json!({"Q": exempt_entry(evidence())}))]});
    let profile = json!({"cases":["a"],"techniques":["B","Q"],"level":3});
    let inventory = result::inventory(&manifest, &profile, &[]).unwrap();
    assert_eq!(
        inventory
            .work
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>(),
        ["B"]
    );
    assert_eq!(inventory.exempt.len(), 1);
    assert_eq!(inventory.exempt[0].1, "Q");
    // The exemption is absent from scheduled work, so it can neither pass nor be counted.
    let expected = ["a-B".to_owned()];
    let results = BTreeMap::from([("a-B".to_owned(), json!({"outcome":"pass"}))]);
    assert!(result::aggregate(&expected, &results, &[], &[]));
    let inflated = BTreeMap::from([
        ("a-B".to_owned(), json!({"outcome":"pass"})),
        ("a-Q".to_owned(), json!({"outcome":"pass"})),
    ]);
    assert!(!result::aggregate(&expected, &inflated, &[], &[]));
}

/// A re-measured control scan of `technique`, detecting it or not.
fn remeasured_scan(technique: &str, detected: bool) -> ScanResult {
    let mut report = clean_report();
    if detected {
        let name = result::TECHNIQUES
            .iter()
            .find(|(id, _)| *id == technique)
            .unwrap()
            .1;
        report["data"] =
            json!([{"type_name":"TECHNIQUES","value":[finding(name, "input", "GET")]}]);
    }
    result::interpret(&report, LOG, Some(0), TARGET, "input", 25)
}

// [spec:pgorm:req:security.sqlmap.profiles+4/test]
// [spec:pgorm:req:security.sqlmap.verdict/test]
#[test]
fn a_detected_but_exempted_pair_fails_the_run() {
    // The guard is real only on a re-measurement, whose results are the declared pairs' own
    // control scans: a profile run scans each control with one scheduled technique.
    let inapplicable = BTreeMap::from([("a-Q".to_owned(), exempt_entry(evidence()))]);
    let held = remeasured_scan("Q", false);
    assert_eq!(result::exemption_outcome(&held, "Q", true), "held");
    let quiet = BTreeMap::from([("a-Q".to_owned(), json!({"outcome":"held","control":held}))]);
    assert!(result::falsified(&quiet, &inapplicable).is_empty());
    assert!(result::remeasured(&["a-Q".into()], &quiet, &[], &[]));
    let fired = remeasured_scan("Q", true);
    assert_eq!(result::exemption_outcome(&fired, "Q", true), "falsified");
    let falsified = BTreeMap::from([(
        "a-Q".to_owned(),
        json!({"outcome":"falsified","control":fired.clone()}),
    )]);
    assert_eq!(result::falsified(&falsified, &inapplicable), ["a-Q"]);
    assert!(!result::remeasured(
        &["a-Q".into()],
        &falsified,
        &[],
        &result::falsified(&falsified, &inapplicable)
    ));
    // A detection falsifies even a scan that did not complete, or followed a failed baseline.
    let mut partial = fired.clone();
    partial.complete = false;
    assert_eq!(result::exemption_outcome(&partial, "Q", false), "falsified");
    // An undetected pair held only if its baseline answered and its scan completed.
    let mut unfinished = held.clone();
    unfinished.complete = false;
    assert_eq!(
        result::exemption_outcome(&unfinished, "Q", true),
        "incomplete"
    );
    assert_eq!(result::exemption_outcome(&held, "Q", false), "incomplete");
    for outcome in ["incomplete", "pass", "falsified"] {
        let results =
            BTreeMap::from([("a-Q".to_owned(), json!({"outcome":outcome,"control":held}))]);
        assert!(
            !result::remeasured(&["a-Q".into()], &results, &[], &[]),
            "{outcome}"
        );
    }
    assert!(!result::remeasured(&[], &BTreeMap::new(), &[], &[]));
    assert!(!result::remeasured(
        &["a-Q".into()],
        &quiet,
        &["container removal failed".into()],
        &[]
    ));
    // A detection on another case, of another technique, or at another injection point falsifies nothing.
    let mut other_parameter = remeasured_scan("Q", true);
    other_parameter.parameter = "probe".into();
    let elsewhere = BTreeMap::from([
        (
            "b-Q".to_owned(),
            json!({"control":remeasured_scan("Q", true)}),
        ),
        (
            "a-U".to_owned(),
            json!({"control":remeasured_scan("U", true)}),
        ),
        ("a-Q".to_owned(), json!({"control":other_parameter})),
    ]);
    assert!(result::falsified(&elsewhere, &inapplicable).is_empty());
}

#[test]
fn command_options_and_encoding_match_python() {
    let manifest: Value = serde_json::from_str(include_str!("../../cases.json")).unwrap();
    let profiles: Value = serde_json::from_str(include_str!("../../profiles.json")).unwrap();
    let mut case = manifest["cases"][0].clone();
    case["baseline"] = json!("a b'+&%");
    assert_eq!(
        harness::target("http://127.0.0.1:12", &case, "control").unwrap(),
        "http://127.0.0.1:12/case/control/select?input=a+b%27%2B%26%25"
    );
    let settings = Settings::from_profile(&profiles["full"]).unwrap();
    let args = harness::scanner_args(
        "python3",
        Path::new("scanner.py"),
        TARGET,
        "B",
        &settings,
        &case,
        Path::new("output"),
    )
    .unwrap();
    assert_eq!(
        args,
        [
            "python3",
            "scanner.py",
            "--url",
            TARGET,
            "-p",
            "input",
            "--dbms",
            "PostgreSQL",
            "--batch",
            "--flush-session",
            "--fresh-queries",
            "--ignore-proxy",
            "--disable-coloring",
            "--technique",
            "B",
            "--level",
            "3",
            "--risk",
            "2",
            "--threads",
            "1",
            "--retries",
            "0",
            "--timeout",
            "15",
            "--time-sec",
            "3",
            "--union-cols",
            "1-4",
            "--output-dir",
            "output/session",
            "--report-json",
            "output/scanner.json",
            "--answers",
            "extending=N,include=N,fuzzy=N",
            "-v",
            "2"
        ]
    );
}

// [spec:pgorm:req:security.sqlmap.profiles+4/test]
#[test]
fn declared_settings_reach_scanner_and_fixture() {
    let manifest: Value = serde_json::from_str(include_str!("../../cases.json")).unwrap();
    let profiles: Value = serde_json::from_str(include_str!("../../profiles.json")).unwrap();
    for name in ["smoke", "full", "identifiers"] {
        Settings::from_profile(&profiles[name]).unwrap();
    }
    // Values no hardcoded default shares, so a setting that stops arriving cannot pass by coincidence.
    let mut profile = profiles["full"].clone();
    for (field, value) in [
        ("level", json!(4)),
        ("risk", json!(1)),
        ("concurrency", json!(7)),
        ("retries", json!(5)),
        ("time_sec", json!(6)),
        ("postgres_statement_timeout_seconds", json!(17)),
        ("postgres_lock_timeout_seconds", json!(3)),
        ("http_timeout_seconds", json!(23)),
        ("case_timeout_seconds", json!(301)),
        ("union_columns", json!([2, 9])),
        ("union_type_fuzzing", json!(true)),
        ("verbosity", json!(3)),
    ] {
        profile[field] = value;
    }
    let settings = Settings::from_profile(&profile).unwrap();
    assert_eq!(settings.case_timeout_seconds, 301);
    let mut case = manifest["cases"][0].clone();
    case["field"] = json!("probe");
    let args = harness::scanner_args(
        "python3",
        Path::new("scanner.py"),
        TARGET,
        "B",
        &settings,
        &case,
        Path::new("output"),
    )
    .unwrap();
    let value = |flag: &str| {
        args[args
            .iter()
            .position(|a| a == flag)
            .unwrap_or_else(|| panic!("{flag} missing"))
            + 1]
        .as_str()
    };
    for (flag, expected) in [
        ("-p", "probe"),
        ("--dbms", "PostgreSQL"),
        ("--level", "4"),
        ("--risk", "1"),
        ("--threads", "7"),
        ("--retries", "5"),
        ("--time-sec", "6"),
        ("--timeout", "23"),
        ("--union-cols", "2-9"),
        ("-v", "3"),
        ("--answers", "extending=N,include=N,fuzzy=Y"),
    ] {
        assert_eq!(value(flag), expected, "{flag}");
    }
    let target = harness::target("http://127.0.0.1:12", &case, "control").unwrap();
    assert!(target.ends_with("?probe=alice"));
    let container = harness::container_args("fixture", "postgres", &settings);
    for setting in ["statement_timeout=17s", "lock_timeout=3s"] {
        assert!(container.contains(&setting.to_owned()), "{setting}");
    }
    let session = harness::role_session("fixture", &settings);
    for setting in ["statement_timeout = '17s'", "lock_timeout = '3s'"] {
        assert!(session.contains(setting), "{setting}");
    }
    // The adapter routes by the same declared parameter, and reads no other.
    let mut renamed = manifest.clone();
    renamed["cases"][0]["field"] = json!("probe");
    let fields = harness::fields(&renamed).unwrap();
    assert_eq!(fields["select"], "probe");
    assert_eq!(fields.len(), manifest["cases"].as_array().unwrap().len());
    let query = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        harness::declared_input(
            &fields["select"],
            &query(&[("probe", "alice"), ("input", "bob")])
        )
        .unwrap(),
        "alice"
    );
    assert!(harness::declared_input(&fields["select"], &query(&[("input", "alice")])).is_err());
    assert!(
        harness::declared_input(
            &fields["select"],
            &query(&[("probe", "alice"), ("probe", "bob")])
        )
        .is_err()
    );
    let mut unnamed = manifest.clone();
    unnamed["cases"][0].as_object_mut().unwrap().remove("field");
    assert!(harness::fields(&unnamed).is_err());
    // The interpreter accepts findings, and completion evidence, only at that parameter.
    let mut report = clean_report();
    report["meta"]["url"] = json!(target.split('?').next());
    report["data"] = json!([{"type_name":"TECHNIQUES","value":[finding("boolean-based blind", "probe", "GET")]}]);
    let probe_log = "GET parameter 'probe' does not seem to be injectable\n[*] ending @ 12:00:00";
    let detected = result::interpret(
        &report,
        probe_log,
        Some(0),
        &target,
        harness::fields(&renamed).unwrap()["select"].as_str(),
        25,
    );
    assert!(detected.complete, "{}", detected.reason);
    assert!(detected.detects("B"));
    let misread = result::interpret(&report, probe_log, Some(0), &target, "input", 25);
    assert!(
        !misread.complete && misread.reason.contains("injection parameter"),
        "{}",
        misread.reason
    );
    assert!(!misread.detects("B"));
    report["data"] = json!([]);
    let untested = result::interpret(&report, LOG, Some(0), &target, "probe", 25);
    assert!(
        !untested.complete && untested.reason.contains("completion evidence"),
        "{}",
        untested.reason
    );
    // A setting the harness would not apply is refused, as is a missing one.
    let mut unknown = profiles["full"].clone();
    unknown["parallel_cases"] = json!(2);
    assert!(Settings::from_profile(&unknown).is_err());
    for field in [
        "dbms",
        "level",
        "risk",
        "concurrency",
        "retries",
        "time_sec",
        "postgres_statement_timeout_seconds",
        "postgres_lock_timeout_seconds",
        "http_timeout_seconds",
        "case_timeout_seconds",
        "union_columns",
        "union_type_fuzzing",
        "verbosity",
        "timeout_margin",
    ] {
        let mut missing = profiles["full"].clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(Settings::from_profile(&missing).is_err(), "{field}");
    }
    for (field, bad) in [
        ("dbms", json!("MySQL")),
        ("postgres_lock_timeout_seconds", json!(0)),
        ("union_columns", json!([4, 1])),
        ("union_columns", json!([0, 4])),
        ("union_columns", json!([1, 4, 5])),
        ("union_columns", json!("1-4")),
        ("union_type_fuzzing", json!("no")),
        ("verbosity", json!(0)),
        ("verbosity", json!(7)),
    ] {
        let mut other = profiles["full"].clone();
        other[field] = bad.clone();
        assert!(Settings::from_profile(&other).is_err(), "{field}={bad}");
    }
    // Timeouts that cannot hold the timed sleep, or that do not nest, are refused.
    for (field, value) in [
        ("time_sec", 10),
        ("postgres_statement_timeout_seconds", 15),
        ("http_timeout_seconds", 240),
    ] {
        let mut tight = profiles["full"].clone();
        tight[field] = json!(value);
        assert!(Settings::from_profile(&tight).is_err(), "{field}");
    }
    let mut unbound = manifest["cases"][0].clone();
    unbound.as_object_mut().unwrap().remove("field");
    assert!(
        harness::scanner_args(
            "python3",
            Path::new("scanner.py"),
            TARGET,
            "B",
            &settings,
            &unbound,
            Path::new("output")
        )
        .is_err()
    );
    assert!(harness::target("http://127.0.0.1:12", &unbound, "control").is_err());
}

#[tokio::test]
async fn process_timeout_and_cancellation_stop_children() {
    let mut child =
        process::Process::spawn(&mut process::command("sh", &["-c", "sleep 120 & wait"])).unwrap();
    assert!(child.wait(1).await.unwrap().is_none());
    assert!(child.child.try_wait().unwrap().is_some());
    let mut child =
        process::Process::spawn(&mut process::command("sh", &["-c", "sleep 120 & wait"])).unwrap();
    child.stop().await.unwrap();
    assert!(child.child.try_wait().unwrap().is_some());
}

// [spec:pgorm:req:security.sqlmap.artifacts/test]
#[test]
fn setup_failure_retains_inventory_and_status() {
    let artifacts = std::env::temp_dir().join(format!("sqlmap-runner-test-{}", std::process::id()));
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_sqlmap-harness"))
        .args([
            "--profile",
            "full",
            "--python",
            "/nonexistent/sqlmap-python",
            "--artifacts",
        ])
        .arg(&artifacts)
        .output()
        .unwrap();
    assert!(!status.status.success());
    let report = harness::read_json(&artifacts.join("report.json")).unwrap();
    assert_eq!(report["expected_scans"].as_array().unwrap().len(), 234);
    assert_eq!(report["results"].as_object().unwrap().len(), 117);
    // Declared inapplicability is retained as evidence even when setup never reaches the scanner.
    assert_eq!(report["inapplicable"].as_object().unwrap().len(), 93);
    assert_eq!(report["pass"], false);
    assert!(report["error"].is_string());
    assert_eq!(report["remeasure"], false);
    assert!(artifacts.join("summary.txt").is_file());
    std::fs::remove_dir_all(artifacts).unwrap();
}

// [spec:pgorm:req:security.sqlmap.profiles+4/test]
// [spec:pgorm:req:security.sqlmap.artifacts/test]
#[test]
fn remeasurement_schedules_exactly_the_declared_pairs() {
    let artifacts =
        std::env::temp_dir().join(format!("sqlmap-remeasure-test-{}", std::process::id()));
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_sqlmap-harness"))
        .args([
            "--profile",
            "full",
            "--remeasure-exemptions",
            "--python",
            "/nonexistent/sqlmap-python",
            "--artifacts",
        ])
        .arg(&artifacts)
        .output()
        .unwrap();
    assert!(!status.status.success());
    let report = harness::read_json(&artifacts.join("report.json")).unwrap();
    assert_eq!(report["remeasure"], true);
    // Each of the 93 declarations is measured on its control alone, and stays recorded as the claim.
    let inapplicable: Vec<_> = report["inapplicable"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(inapplicable.len(), 93);
    let mut expected: Vec<_> = report["expected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap().to_owned())
        .collect();
    expected.sort();
    assert_eq!(expected, inapplicable);
    let scans = report["expected_scans"].as_array().unwrap();
    assert_eq!(scans.len(), 93);
    assert!(
        scans
            .iter()
            .all(|s| s.as_str().unwrap().ends_with("-control"))
    );
    assert_eq!(report["pass"], false);
    assert!(
        std::fs::read_to_string(artifacts.join("summary.txt"))
            .unwrap()
            .starts_with("full exemption re-measurement: FAIL")
    );
    std::fs::remove_dir_all(artifacts).unwrap();
}

// [spec:pgorm:req:security.sqlmap.pin+1/test]
#[test]
fn python_takes_any_patch_of_the_pinned_series() {
    let pins: Value = serde_json::from_str(include_str!("../../pins.json")).unwrap();
    assert_eq!(pins["python"], "3.14");
    for (reported, release) in [
        ("Python 3.14.0", "3.14.0"),
        ("Python 3.14.4\n", "3.14.4"),
        ("Python 3.14.12", "3.14.12"),
        ("Python 3.14.1rc1", "3.14.1rc1"),
    ] {
        assert_eq!(
            harness::python_release(reported, "3.14").unwrap(),
            release,
            "{reported}"
        );
    }
    for reported in [
        "Python 3.13.9",
        "Python 3.15.0",
        "Python 3.140.1",
        "Python 3.1.4",
        "Python 3.14",
        "Python 3.14.",
        "PyPy 3.14.4",
        "3.14.4",
    ] {
        assert!(
            harness::python_release(reported, "3.14").is_err(),
            "{reported}"
        );
    }
    // The pin names a series, never a patch release or a range.
    for series in ["3.14.4", "3", ">=3.14", "3.x", ""] {
        assert!(
            harness::python_release("Python 3.14.4", series).is_err(),
            "{series}"
        );
    }
}

#[test]
fn cli_preserves_profile_subset_and_diagnostic_options() {
    let options = Options::parse(
        [
            "--profile",
            "full",
            "--case",
            "select",
            "--baseline-only",
            "--direct-regressions",
        ]
        .into_iter()
        .map(str::to_owned),
        Path::new("/tmp"),
    )
    .unwrap();
    assert_eq!(options.profile, "full");
    assert_eq!(options.subset, vec!["select"]);
    assert!(options.baseline_only && options.direct_regressions);
    assert!(Options::parse(["--profile".into()].into_iter(), Path::new("/tmp")).is_err());
    let identifiers = Options::parse(
        ["--profile", "identifiers"].into_iter().map(str::to_owned),
        Path::new("/tmp"),
    )
    .unwrap();
    assert_eq!(identifiers.profile, "identifiers");
    assert!(
        Options::parse(
            ["--profile", "nonsense"].into_iter().map(str::to_owned),
            Path::new("/tmp")
        )
        .is_err()
    );
    assert!(!identifiers.remeasure);
    let remeasure = Options::parse(
        [
            "--profile",
            "full",
            "--case",
            "select",
            "--remeasure-exemptions",
        ]
        .into_iter()
        .map(str::to_owned),
        Path::new("/tmp"),
    )
    .unwrap();
    assert!(remeasure.remeasure);
    assert_eq!(remeasure.subset, vec!["select"]);
    for other in ["--baseline-only", "--direct-regressions"] {
        assert!(
            Options::parse(
                ["--remeasure-exemptions", other]
                    .into_iter()
                    .map(str::to_owned),
                Path::new("/tmp")
            )
            .is_err(),
            "{other}"
        );
    }
}

/// Optional corpus check: reinterprets every retained Python scan, including failures.
#[test]
fn parity_against_python_campaign() {
    let Some(artifact_dir) = std::env::var_os("SQLMAP_PARITY_REPORT") else {
        return;
    };
    let artifact_dir = PathBuf::from(artifact_dir);
    let report = harness::read_json(&artifact_dir.join("report.json")).unwrap();
    let mut checked = 0;
    for key in report["expected"].as_array().unwrap() {
        for mode in ["control", "protected"] {
            let output = artifact_dir.join(format!("{}-{mode}", key.as_str().unwrap()));
            let expected = harness::read_json(&output.join("result.json")).unwrap();
            let scanner = harness::read_json(&output.join("scanner.json")).unwrap_or(Value::Null);
            let log = std::fs::read_to_string(output.join("scanner.log")).unwrap();
            let args = harness::read_json(&output.join("command.json")).unwrap();
            let args = args.as_array().unwrap();
            let target = args[args.iter().position(|a| a == "--url").unwrap() + 1]
                .as_str()
                .unwrap();
            let parameter = args[args.iter().position(|a| a == "-p").unwrap() + 1]
                .as_str()
                .unwrap();
            let actual = result::interpret(
                &scanner,
                &log,
                expected["returncode"].as_i64().map(|c| c as i32),
                target,
                parameter,
                expected["requests"].as_u64().unwrap(),
            );
            assert_eq!(
                actual.complete,
                expected["complete"].as_bool().unwrap(),
                "{}",
                output.display()
            );
            assert_eq!(
                json!(actual.findings),
                expected["findings"],
                "{}",
                output.display()
            );
            assert_eq!(
                json!(actual.errors),
                expected["errors"],
                "{}",
                output.display()
            );
            assert_eq!(
                actual.reason,
                expected["reason"].as_str().unwrap(),
                "{}",
                output.display()
            );
            checked += 1;
        }
    }
    // The corpus must be a complete full-profile campaign, whose size depends on how
    // many pairs the manifest declares inapplicable at the revision that produced it.
    assert_eq!(report["profile"], "full");
    assert!(report["subset"].as_array().unwrap().is_empty());
    assert_eq!(checked, report["expected"].as_array().unwrap().len() * 2);
    assert!(checked >= 252, "{checked}");
}
