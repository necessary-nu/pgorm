//! CI must not turn missing, partial or failing runner evidence into a pass.
use pgorm_sqlmap_adapter::harness::verdict;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn scratch(name: &str) -> PathBuf {
    let artifacts =
        std::env::temp_dir().join(format!("sqlmap-verdict-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&artifacts);
    fs::create_dir_all(artifacts.join("run")).unwrap();
    artifacts
}

fn passing() -> Value {
    json!({"profile": "full", "subset": [], "remeasure": false, "pass": true, "direct_regressions": {"pass": true},
           "inapplicable": {"select-Q": {"reason": "REPLACE-only and never reflected here."}},
           "falsified_exemptions": []})
}

// [spec:pgorm:req:security.sqlmap.ci+2/test]
#[test]
fn missing_report_is_explicitly_not_run() {
    let artifacts = scratch("missing");
    assert_eq!(
        verdict::profile_status("smoke", &artifacts).status,
        "not-run"
    );
}

// [spec:pgorm:req:security.sqlmap.ci+2/test]
#[test]
fn profile_status_requires_matching_complete_evidence() {
    let artifacts = scratch("evidence");
    let report = artifacts.join("run/report.json");
    let cases = [
        (None, "pass"),
        (Some(("pass", json!(false))), "fail"),
        (Some(("subset", json!(["select"]))), "incomplete"),
        (Some(("profile", json!("smoke"))), "incomplete"),
        (
            Some(("direct_regressions", json!({"pass": false}))),
            "incomplete",
        ),
        (Some(("direct_regressions", Value::Null)), "incomplete"),
        (Some(("falsified_exemptions", json!(["select-Q"]))), "fail"),
        (Some(("inapplicable", Value::Null)), "incomplete"),
        (Some(("falsified_exemptions", Value::Null)), "incomplete"),
        // A re-measurement of the declarations, however it ended, is not a profile run.
        (Some(("remeasure", json!(true))), "incomplete"),
        (Some(("remeasure", Value::Null)), "incomplete"),
    ];
    for (update, expected) in cases {
        let mut document = passing();
        if let Some((field, replacement)) = &update {
            document[field] = replacement.clone();
        }
        fs::write(&report, document.to_string()).unwrap();
        assert_eq!(
            verdict::profile_status("full", &artifacts).status,
            expected,
            "{update:?}"
        );
    }
    for malformed in ["{", "[]", "null"] {
        fs::write(&report, malformed).unwrap();
        assert_eq!(
            verdict::profile_status("full", &artifacts).status,
            "incomplete",
            "{malformed}"
        );
    }
}

// [spec:pgorm:req:security.sqlmap.ci+2/test]
// [spec:pgorm:req:security.sqlmap.profiles+4/test]
#[test]
fn exemptions_are_published_and_never_absorbed() {
    let artifacts = scratch("exemptions");
    fs::write(artifacts.join("run/report.json"), passing().to_string()).unwrap();
    let status = verdict::profile_status("full", &artifacts);
    assert_eq!(
        (
            status.status,
            status
                .inapplicable
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
        ),
        ("pass", vec!["select-Q"])
    );
    assert!(
        status.reason.contains("1 pairs are declared inapplicable"),
        "{}",
        status.reason
    );

    let (passed, summary) = verdict::publish("full", &artifacts).unwrap();
    assert!(passed);
    assert!(summary.starts_with("sqlmap / full: PASS\n"), "{summary}");
    assert!(
        summary.contains("- select-Q: REPLACE-only and never reflected here.\n"),
        "{summary}"
    );
    assert_eq!(
        fs::read_to_string(artifacts.join("ci-summary.txt")).unwrap(),
        summary
    );
    let published: Value =
        serde_json::from_str(&fs::read_to_string(artifacts.join("ci-status.json")).unwrap())
            .unwrap();
    assert_eq!(published["status"], "pass");
    assert_eq!(
        published["inapplicable"]["select-Q"],
        "REPLACE-only and never reflected here."
    );
}

// [spec:pgorm:req:security.sqlmap.ci+2/test]
#[test]
fn a_failing_profile_still_publishes_its_exemptions() {
    let artifacts = scratch("failing");
    let mut document = passing();
    document["pass"] = json!(false);
    fs::write(artifacts.join("run/report.json"), document.to_string()).unwrap();
    let (passed, summary) = verdict::publish("full", &artifacts).unwrap();
    assert!(!passed);
    assert!(summary.starts_with("sqlmap / full: FAIL\n"), "{summary}");
    assert!(summary.contains("- select-Q: "), "{summary}");
}

// [spec:pgorm:req:security.sqlmap.ci+2/test]
#[test]
fn publish_refuses_an_unknown_profile() {
    assert!(verdict::publish("nonsense", &scratch("unknown")).is_err());
}

// [spec:pgorm:req:security.sqlmap.ci+2/test]
#[test]
fn identifiers_status_needs_no_direct_regressions() {
    let artifacts = scratch("identifiers");
    let mut document = passing();
    document["profile"] = json!("identifiers");
    document
        .as_object_mut()
        .unwrap()
        .remove("direct_regressions");
    fs::write(artifacts.join("run/report.json"), document.to_string()).unwrap();
    // Direct regressions belong to the full profile; the identifiers scan neither runs nor needs them.
    let (passed, summary) = verdict::publish("identifiers", &artifacts).unwrap();
    assert!(passed, "{summary}");
    assert!(
        summary.starts_with("sqlmap / identifiers: PASS\n"),
        "{summary}"
    );
    // A full-profile report cannot stand in for it, nor it for the full profile.
    assert_eq!(
        verdict::profile_status("full", &artifacts).status,
        "incomplete"
    );
    fs::write(artifacts.join("run/report.json"), passing().to_string()).unwrap();
    assert_eq!(
        verdict::profile_status("identifiers", &artifacts).status,
        "incomplete"
    );
}
