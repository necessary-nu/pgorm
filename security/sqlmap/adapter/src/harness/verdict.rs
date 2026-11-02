//! Publish a profile's runner verdict, including an explicit not-run status.
use super::{Result, write_json};
use serde_json::{Map, Value, json};
use std::{fs, path::Path};

/// A published verdict: the status, why it was reached, and the exemptions the
/// report declared. Exemptions travel with the status so a failing or
/// incomplete run still publishes what it had claimed was out of scope.
#[derive(Debug)]
pub struct Status {
    pub status: &'static str,
    pub reason: String,
    pub inapplicable: Map<String, Value>,
}

// [spec:pgorm:req:security.sqlmap.ci+2]
// [spec:pgorm:req:security.sqlmap.profiles+4]
pub fn profile_status(profile: &str, artifacts: &Path) -> Status {
    let bare = |status: &'static str, reason: &str| Status {
        status,
        reason: reason.into(),
        inapplicable: Map::new(),
    };
    let bytes = match fs::read(artifacts.join("run/report.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return bare("not-run", "setup failed before the runner wrote its report");
        }
        Err(_) => return bare("incomplete", "runner report is unreadable"),
    };
    let Ok(document) = serde_json::from_slice::<Value>(&bytes) else {
        return bare("incomplete", "runner report is unreadable");
    };
    let Some(report) = document.as_object() else {
        return bare("incomplete", "runner report is malformed");
    };
    if report.get("profile") != Some(&json!(profile)) || report.get("subset") != Some(&json!([])) {
        return bare(
            "incomplete",
            "report does not cover the requested complete profile",
        );
    }
    // A re-measurement scans the declared-inapplicable pairs instead of the scheduled work,
    // so even a passing one says nothing about the profile.
    if report.get("remeasure") != Some(&json!(false)) {
        return bare(
            "incomplete",
            "report is not a profile run: it re-measures declared inapplicability or does not say",
        );
    }
    // A report that does not account for its own exemptions cannot be audited against
    // the work it claims to have scheduled, so it is evidence of nothing.
    let (Some(inapplicable), Some(falsified)) = (
        report.get("inapplicable").and_then(Value::as_object),
        report.get("falsified_exemptions").and_then(Value::as_array),
    ) else {
        return bare(
            "incomplete",
            "report does not account for declared inapplicability",
        );
    };
    let carried = |status: &'static str, reason: String| Status {
        status,
        reason,
        inapplicable: inapplicable.clone(),
    };
    if !falsified.is_empty() {
        let mut pairs: Vec<String> = falsified
            .iter()
            .map(|pair| {
                pair.as_str()
                    .map_or_else(|| pair.to_string(), str::to_owned)
            })
            .collect();
        pairs.sort();
        return carried(
            "fail",
            format!(
                "the scanner detected declared-inapplicable pairs: {}",
                pairs.join(", ")
            ),
        );
    }
    if report.get("pass") != Some(&json!(true)) {
        return carried(
            "fail",
            "runner did not report a passing profile; inspect run/report.json".into(),
        );
    }
    let regressions = report.get("direct_regressions").and_then(Value::as_object);
    if profile == "full"
        && !regressions.is_some_and(|direct| direct.get("pass") == Some(&json!(true)))
    {
        return carried(
            "incomplete",
            "full profile lacks passing direct security regressions".into(),
        );
    }
    carried(
        "pass",
        format!(
            "the pinned profile passed every scheduled pair; {} pairs are declared inapplicable",
            inapplicable.len()
        ),
    )
}

/// Writes `ci-status.json` and `ci-summary.txt`, and reports whether the profile
/// passed alongside the rendered summary. Echoing that summary anywhere else is
/// the caller's business, so publishing stays free of ambient environment.
// [spec:pgorm:req:security.sqlmap.ci+2]
pub fn publish(profile: &str, artifacts: &Path) -> Result<(bool, String)> {
    if !["smoke", "full", "identifiers"].contains(&profile) {
        return Err("profile must be smoke, full or identifiers".into());
    }
    fs::create_dir_all(artifacts)?;
    let Status {
        status,
        reason,
        inapplicable,
    } = profile_status(profile, artifacts);
    // Only each exemption's reason is published: the full evidence block belongs to the
    // report, and a CI summary that reprinted it would bury the verdict it exists to state.
    let declared: Map<String, Value> = inapplicable
        .iter()
        .filter_map(|(pair, entry)| {
            Some((
                pair.clone(),
                entry
                    .as_object()?
                    .get("reason")
                    .cloned()
                    .unwrap_or(Value::Null),
            ))
        })
        .collect();
    write_json(
        &artifacts.join("ci-status.json"),
        &json!({"profile": profile, "status": status, "reason": reason, "inapplicable": declared}),
    )?;
    let mut summary = format!(
        "sqlmap / {profile}: {}\n\n{reason}\n",
        status.to_uppercase()
    );
    if !declared.is_empty() {
        summary.push_str(&format!("\nDeclared inapplicable ({} pairs, excluded from scheduled work and never counted as passes):\n", declared.len()));
        for (pair, why) in &declared {
            summary.push_str(&format!("- {pair}: {}\n", why.as_str().unwrap_or_default()));
        }
    }
    fs::write(artifacts.join("ci-summary.txt"), &summary)?;
    Ok((status == "pass", summary))
}
