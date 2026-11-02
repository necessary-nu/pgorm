use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

pub const CLEAN_MESSAGE: &str = "all tested parameters do not appear to be injectable.";
pub const TECHNIQUES: [(&str, &str); 6] = [
    ("B", "boolean-based blind"),
    ("E", "error-based"),
    ("U", "UNION query"),
    ("S", "stacked queries"),
    ("T", "time-based blind"),
    ("Q", "inline query"),
];
/// The payload definition that supplies each technique's tests. An exemption must cite
/// its own technique's file so a reviewer lands on the `<test>` entries being claimed about.
pub const PAYLOADS: [(&str, &str); 6] = [
    ("B", "boolean_blind.xml"),
    ("E", "error_based.xml"),
    ("U", "union_query.xml"),
    ("S", "stacked_queries.xml"),
    ("T", "time_blind.xml"),
    ("Q", "inline_query.xml"),
];
const EVIDENCE_FIELDS: [&str; 6] = ["boundary", "clause", "contexts", "kind", "payload", "where"];
/// The scanner level at which a boundary first reaches the context. Below it the pair is
/// inapplicable; a profile at or above it schedules the pair, so the declaration cannot
/// excuse a stronger scan from work it has been measured to reach.
const CEILING_FIELD: &str = "ceiling";
const EVIDENCE_KINDS: [&str; 2] = ["no-attachable-position", "no-boundary"];
// Prose floors that refuse "n/a" without pretending to judge the argument itself.
const REASON_CHARS: usize = 40;
const BOUNDARY_CHARS: usize = 20;

/// Work list plus the pairs an evidenced manifest declaration removed from it.
#[derive(Debug, Default)]
pub struct Inventory {
    pub work: Vec<(Value, String)>,
    pub exempt: Vec<(String, String, Value)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub complete: bool,
    pub reason: String,
    /// The case's declared test parameter: the only injection point a finding may name.
    pub parameter: String,
    pub findings: Vec<Value>,
    pub errors: Vec<Value>,
    pub requests: u64,
    pub returncode: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
}

impl ScanResult {
    /// Whether a well-formed finding at the declared parameter shows `technique`.
    pub fn detects(&self, technique: &str) -> bool {
        findings_show_technique(&self.findings, technique, &self.parameter)
    }
}

fn valid_finding(finding: &Value, parameter: &str) -> bool {
    finding["parameter"] == parameter
        && finding["place"] == "GET"
        && finding["data"].as_array().is_some_and(|data| {
            !data.is_empty()
                && data
                    .iter()
                    .all(|d| TECHNIQUES.iter().any(|(_, name)| d["technique"] == *name))
        })
}

/// Findings count only at `parameter`, the case's declared test parameter, and only its own
/// "does not seem to be injectable" line is evidence the scanner tested it.
// [spec:pgorm:req:security.sqlmap.execution]
// [spec:pgorm:req:security.sqlmap.profiles+4]
pub fn interpret(
    report: &Value,
    log: &str,
    returncode: Option<i32>,
    target: &str,
    parameter: &str,
    requests: u64,
) -> ScanResult {
    let mut findings = Vec::new();
    let mut failures = Vec::new();
    let mut malformed = !report["data"].is_array() || !report["error"].is_array();
    if let Some(data) = report["data"].as_array() {
        for entry in data {
            if !entry.is_object() {
                malformed = true;
            } else if entry["type_name"] == "TECHNIQUES" {
                if let Some(values) = entry["value"].as_array() {
                    findings.extend(values.iter().cloned());
                } else {
                    malformed = true;
                }
            }
        }
    }
    let errors: Vec<_> = report["error"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| !e.as_str().is_some_and(|s| s.starts_with(CLEAN_MESSAGE)))
        .cloned()
        .collect();
    if returncode != Some(0) {
        failures.push(match returncode {
            Some(code) => format!("scanner exited {code}"),
            None => "scanner timed out or cancelled".into(),
        });
    }
    if report["success"] != true {
        failures.push("scanner result missing or unsuccessful".into());
    }
    if malformed {
        failures.push("malformed scanner data".into());
    }
    // sqlmap's meta.url omits the query; the injection point is checked separately.
    match (
        report["meta"]["url"]
            .as_str()
            .and_then(|s| Url::parse(s).ok()),
        Url::parse(target),
    ) {
        (Some(actual), Ok(wanted)) => {
            for (field, matches) in [
                ("scheme", actual.scheme() == wanted.scheme()),
                ("hostname", actual.host_str() == wanted.host_str()),
                (
                    "port",
                    actual.port_or_known_default() == wanted.port_or_known_default(),
                ),
                ("path", actual.path() == wanted.path()),
            ] {
                if !matches {
                    failures.push(format!("target {field} mismatch"));
                }
            }
        }
        _ => failures.push("malformed scanner URL".into()),
    }
    if requests == 0 {
        failures.push("inactive route: no accounted requests".into());
    }
    let valid = findings
        .iter()
        .filter(|f| valid_finding(f, parameter))
        .count();
    if valid != findings.len() {
        failures.push("unexpected injection parameter or malformed finding".into());
    }
    if !errors.is_empty() {
        failures.push("scanner reported errors".into());
    }
    let lower = log.to_lowercase();
    if [
        "connection timed out",
        "unable to connect",
        "connection reset",
        "user aborted",
        "skipping parameter",
    ]
    .iter()
    .any(|s| lower.contains(s))
    {
        failures.push("transport failure or skipped parameter".into());
    }
    let tested = log.contains(&format!(
        "parameter '{parameter}' does not seem to be injectable"
    )) || valid > 0;
    if !tested || !log.contains("[*] ending @") {
        failures.push("missing completion evidence".into());
    }
    ScanResult {
        complete: failures.is_empty(),
        reason: if failures.is_empty() {
            "completed".into()
        } else {
            failures.join("; ")
        },
        parameter: parameter.into(),
        findings,
        errors,
        requests,
        returncode,
        seconds: None,
    }
}

pub fn findings_show_technique(findings: &[Value], technique: &str, parameter: &str) -> bool {
    TECHNIQUES
        .iter()
        .find(|(id, _)| *id == technique)
        .is_some_and(|(_, name)| {
            findings.iter().any(|f| {
                valid_finding(f, parameter)
                    && f["data"]
                        .as_array()
                        .is_some_and(|data| data.iter().any(|d| d["technique"] == *name))
            })
        })
}

// [spec:pgorm:req:security.sqlmap.outcomes]
pub fn verdict(
    control: &ScanResult,
    protected: &ScanResult,
    technique: &str,
    baseline: bool,
    invariant: bool,
) -> &'static str {
    if !protected.findings.is_empty() || !invariant {
        return "vulnerable";
    }
    if !baseline || !control.complete {
        return "incomplete";
    }
    if !control.detects(technique) {
        return "invalid-control";
    }
    if !protected.complete {
        return "incomplete";
    }
    "pass"
}

/// A re-measured declaration. Any detection of the exempted technique falsifies it, even from
/// a scan that did not otherwise complete, just as a protected finding is never excused; it
/// held only when the control answered its baseline and its scan completed without one.
// [spec:pgorm:req:security.sqlmap.profiles+4]
pub fn exemption_outcome(control: &ScanResult, technique: &str, baseline: bool) -> &'static str {
    if control.detects(technique) {
        return "falsified";
    }
    if !baseline || !control.complete {
        return "incomplete";
    }
    "held"
}

fn case_of(key: &str) -> Option<&str> {
    key.rsplit_once('-').map(|(case, _)| case)
}

/// A technique any retained control scan of the case detected was never inapplicable for it.
/// A profile run scans each control with one scheduled technique only, so this fires on a
/// re-measurement run, whose results are the declared pairs' own control scans.
// [spec:pgorm:req:security.sqlmap.profiles+4]
pub fn falsified(
    results: &BTreeMap<String, Value>,
    inapplicable: &BTreeMap<String, Value>,
) -> Vec<String> {
    let mut hits = Vec::new();
    for key in inapplicable.keys() {
        let Some((case, technique)) = key.rsplit_once('-') else {
            continue;
        };
        if results.iter().any(|(scheduled, result)| {
            case_of(scheduled) == Some(case)
                && result["control"]["parameter"]
                    .as_str()
                    .is_some_and(|parameter| {
                        findings_show_technique(
                            result["control"]["findings"]
                                .as_array()
                                .map_or(&[][..], Vec::as_slice),
                            technique,
                            parameter,
                        )
                    })
        }) {
            hits.push(key.clone());
        }
    }
    hits
}

/// Every expected key accounted for exactly once, each ending `required`, with clean
/// fixture teardown and no falsified declaration.
fn accounted(
    expected: &[String],
    results: &BTreeMap<String, Value>,
    cleanup: &[String],
    falsified: &[String],
    required: &str,
) -> bool {
    let unique: BTreeSet<_> = expected.iter().collect();
    !expected.is_empty()
        && expected.len() == unique.len()
        && unique == results.keys().collect()
        && results.values().all(|r| r["outcome"] == required)
        && cleanup.is_empty()
        && falsified.is_empty()
}

// [spec:pgorm:req:security.sqlmap.verdict]
pub fn aggregate(
    expected: &[String],
    results: &BTreeMap<String, Value>,
    cleanup: &[String],
    falsified: &[String],
) -> bool {
    accounted(expected, results, cleanup, falsified, "pass")
}

/// A re-measurement passes only when every declared pair it scanned held.
// [spec:pgorm:req:security.sqlmap.profiles+4]
pub fn remeasured(
    expected: &[String],
    results: &BTreeMap<String, Value>,
    cleanup: &[String],
    falsified: &[String],
) -> bool {
    accounted(expected, results, cleanup, falsified, "held")
}

/// A CONTEXTS.md section anchor: a decimal number with an optional subsection letter.
fn section(text: &str) -> bool {
    let body = match text.chars().next_back() {
        Some(last) if last.is_ascii_lowercase() => &text[..text.len() - last.len_utf8()],
        _ => text,
    };
    !body.is_empty() && !body.starts_with('0') && body.chars().all(|c| c.is_ascii_digit())
}

fn strings(value: &Value) -> Result<Vec<String>, String> {
    value
        .as_array()
        .ok_or("missing inventory list")?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or("non-string inventory entry".into())
        })
        .collect()
}

/// Declared inapplicability, refused unless it cites checkable scanner evidence.
// [spec:pgorm:req:security.sqlmap.profiles+4]
pub fn exemptions(case: &Value) -> Result<BTreeMap<String, Value>, String> {
    let id = case["id"].as_str().ok_or("missing case id")?;
    let declared = match &case["inapplicable"] {
        Value::Null => return Ok(BTreeMap::new()),
        Value::Object(map) => map,
        _ => return Err(format!("malformed inapplicable block for {id}")),
    };
    let techniques = strings(&case["techniques"])?;
    let mut out = BTreeMap::new();
    for (technique, entry) in declared {
        let pair = format!("{id}-{technique}");
        let payload = PAYLOADS
            .iter()
            .find(|(t, _)| t == technique)
            .ok_or(format!("inapplicable {pair} names an unknown technique"))?
            .1;
        if !techniques.contains(technique) {
            return Err(format!(
                "inapplicable {pair} names a technique the case does not declare"
            ));
        }
        if entry["reason"]
            .as_str()
            .map_or(0, |r| r.trim().chars().count())
            < REASON_CHARS
        {
            return Err(format!("inapplicable {pair} needs a substantive reason"));
        }
        let evidence = entry["evidence"].as_object().ok_or(format!(
            "inapplicable {pair} needs evidence fields {EVIDENCE_FIELDS:?}"
        ))?;
        if evidence
            .keys()
            .map(String::as_str)
            .filter(|k| *k != CEILING_FIELD)
            .collect::<BTreeSet<_>>()
            != EVIDENCE_FIELDS.into_iter().collect()
        {
            return Err(format!(
                "inapplicable {pair} needs evidence fields {EVIDENCE_FIELDS:?}"
            ));
        }
        // sqlmap's levels run 1 to 5, and a ceiling of 1 would exempt the pair at no level at all.
        if evidence
            .get(CEILING_FIELD)
            .is_some_and(|c| c.as_u64().is_none_or(|n| !(2..=5).contains(&n)))
        {
            return Err(format!(
                "inapplicable {pair} has a ceiling that is not a scanner level from 2 to 5"
            ));
        }
        if !EVIDENCE_KINDS.iter().any(|k| evidence["kind"] == *k) {
            return Err(format!("inapplicable {pair} has an unknown evidence kind"));
        }
        if evidence["payload"] != payload {
            return Err(format!(
                "inapplicable {pair} cites a payload that does not define {technique}"
            ));
        }
        for (field, high) in [("where", 3u64), ("clause", 9)] {
            let values = evidence[field].as_array().ok_or(format!(
                "inapplicable {pair} has malformed payload {field} values"
            ))?;
            if values.is_empty()
                || values.iter().any(|v| {
                    v.as_u64()
                        .is_none_or(|n| n > high || (field == "where" && n == 0))
                })
            {
                return Err(format!(
                    "inapplicable {pair} has malformed payload {field} values"
                ));
            }
        }
        if evidence["boundary"]
            .as_str()
            .map_or(0, |b| b.trim().chars().count())
            < BOUNDARY_CHARS
        {
            return Err(format!(
                "inapplicable {pair} must name the boundary its context would require"
            ));
        }
        if !evidence["contexts"].as_str().is_some_and(section) {
            return Err(format!(
                "inapplicable {pair} must cite a CONTEXTS.md section"
            ));
        }
        out.insert(technique.clone(), entry.clone());
    }
    Ok(out)
}

// [spec:pgorm:req:security.sqlmap.profiles+4]
pub fn inventory(
    manifest: &Value,
    profile: &Value,
    subset: &[String],
) -> Result<Inventory, String> {
    let cases = manifest["cases"]
        .as_array()
        .ok_or("missing manifest cases")?;
    let mut by_id = BTreeMap::new();
    for case in cases {
        let id = case["id"].as_str().ok_or("missing case id")?;
        if by_id.insert(id.to_owned(), case).is_some() {
            return Err("duplicated manifest case".into());
        }
    }
    let selected = strings(&profile["cases"])?;
    if subset.iter().any(|s| !selected.contains(s)) {
        return Err("subset contains cases outside the selected profile".into());
    }
    let selected = if subset.is_empty() {
        selected
    } else {
        subset.to_vec()
    };
    if selected.is_empty() || selected.len() != selected.iter().collect::<BTreeSet<_>>().len() {
        return Err("empty or duplicated case inventory".into());
    }
    let techniques = strings(&profile["techniques"])?;
    if techniques.is_empty()
        || techniques.len() != techniques.iter().collect::<BTreeSet<_>>().len()
        || techniques
            .iter()
            .any(|t| !TECHNIQUES.iter().any(|(id, _)| id == t))
    {
        return Err("empty, duplicated or unknown technique inventory".into());
    }
    let level = profile["level"]
        .as_u64()
        .ok_or("profile must declare its scanner level")?;
    let mut inventory = Inventory::default();
    for id in selected {
        let case = by_id.get(&id).ok_or(format!("unknown case {id}"))?;
        let enabled = strings(&case["techniques"])?;
        if enabled.len() != enabled.iter().collect::<BTreeSet<_>>().len() {
            return Err("duplicated case technique".into());
        }
        let offered: Vec<_> = enabled
            .into_iter()
            .filter(|t| techniques.contains(t))
            .collect();
        // A case the profile drops outright is still unexplained work; only an
        // evidenced exemption may remove a technique the profile does offer.
        if offered.is_empty() {
            return Err(format!("no scheduled techniques for {id}"));
        }
        let declared = exemptions(case)?;
        for technique in offered {
            match declared.get(&technique) {
                Some(entry)
                    if entry["evidence"][CEILING_FIELD]
                        .as_u64()
                        .is_none_or(|ceiling| level < ceiling) =>
                {
                    inventory
                        .exempt
                        .push((id.clone(), technique, entry.clone()))
                }
                _ => inventory.work.push(((*case).clone(), technique)),
            }
        }
    }
    Ok(inventory)
}
