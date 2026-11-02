mod fixture;
pub mod process;
pub mod result;
pub mod verdict;

use fixture::Fixture;
pub use fixture::{container_args, role_session};
use result::{
    ScanResult, aggregate, exemption_outcome, falsified, interpret, inventory, remeasured, verdict,
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::time::Instant;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn read_json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

pub fn write_json(path: &Path, data: &impl Serialize) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        format!("{}\n", serde_json::to_string_pretty(data)?),
    )?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn hash(bytes: impl AsRef<[u8]>) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn digest(path: &Path) -> Result<String> {
    Ok(hash(fs::read(path)?))
}

#[derive(Debug)]
pub struct Options {
    pub profile: String,
    pub subset: Vec<String>,
    pub artifacts: PathBuf,
    pub baseline_only: bool,
    pub direct_regressions: bool,
    /// Scan each declared-inapplicable pair's control with its exempted technique instead of
    /// the profile's scheduled work, expecting no detection.
    pub remeasure: bool,
    pub python: String,
}

impl Options {
    pub fn parse(args: impl Iterator<Item = String>, root: &Path) -> Result<Self> {
        let mut options = Self {
            profile: "smoke".into(),
            subset: Vec::new(),
            artifacts: root.join("target/sqlmap").join(format!(
                "rust-{}-{}",
                SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                std::process::id()
            )),
            baseline_only: false,
            direct_regressions: false,
            remeasure: false,
            python: "python3".into(),
        };
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            let mut value = || {
                args.next()
                    .ok_or_else(|| format!("missing value for {arg}"))
            };
            match arg.as_str() {
                "--profile" => options.profile = value()?,
                "--case" => options.subset.push(value()?),
                "--artifacts" => options.artifacts = value()?.into(),
                "--python" => options.python = value()?,
                "--baseline-only" => options.baseline_only = true,
                "--direct-regressions" => options.direct_regressions = true,
                "--remeasure-exemptions" => options.remeasure = true,
                _ => return Err(format!("unknown option: {arg}").into()),
            }
        }
        if !["smoke", "full", "identifiers"].contains(&options.profile.as_str()) {
            return Err("profile must be smoke, full or identifiers".into());
        }
        if options.remeasure && (options.baseline_only || options.direct_regressions) {
            return Err("--remeasure-exemptions scans only the declared-inapplicable pairs; it takes neither --baseline-only nor --direct-regressions".into());
        }
        if !options.artifacts.is_absolute() {
            options.artifacts = std::env::current_dir()?.join(options.artifacts);
        }
        Ok(options)
    }
}

// [spec:pgorm:req:security.sqlmap.artifacts]
async fn source_identity(root: &Path) -> Result<Value> {
    let mut command = process::command("git", &["ls-files", "-co", "--exclude-standard", "-z"]);
    command.current_dir(root);
    let paths = process::output(&mut command, None, 120).await?;
    let mut files = BTreeMap::new();
    for path in paths.split('\0').filter(|p| !p.is_empty()) {
        let file = root.join(path);
        if file.is_file() && !path.starts_with(".nplan/") && !path.starts_with("plan/") {
            files.insert(path.to_owned(), digest(&file)?);
        }
    }
    let mut command = process::command("git", &["rev-parse", "HEAD"]);
    command.current_dir(root);
    let head = process::output(&mut command, None, 120).await?;
    let digest = hash(serde_json::to_vec(&files)?);
    Ok(json!({"head":head,"files":files,"digest":digest}))
}

/// The release an interpreter's `--version` line reports, refused unless it belongs to the
/// minor series `pins.json` names. Any patch of the series is accepted, so CI takes its latest;
/// the run records the release it used instead of requiring one.
// [spec:pgorm:req:security.sqlmap.pin+1]
pub fn python_release(reported: &str, series: &str) -> Result<String> {
    let mut parts = series.split('.');
    let numeric = |part: Option<&str>| {
        part.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    if !(numeric(parts.next()) && numeric(parts.next()) && parts.next().is_none()) {
        return Err(
            format!("pins.json python must name a major.minor series, not {series:?}").into(),
        );
    }
    let release = reported
        .trim()
        .strip_prefix("Python ")
        .ok_or_else(|| format!("unrecognised interpreter version report: {reported:?}"))?;
    match release
        .strip_prefix(series)
        .and_then(|rest| rest.strip_prefix('.'))
    {
        Some(patch) if patch.starts_with(|c: char| c.is_ascii_digit()) => Ok(release.to_owned()),
        _ => Err(format!("Python {series} required; got {release}").into()),
    }
}

// [spec:pgorm:req:security.sqlmap.pin+1]
async fn scanner(pins: &Value, cache: &Path, artifacts: &Path) -> Result<PathBuf> {
    fs::create_dir_all(cache)?;
    let archive = cache.join("sqlmap.tar.gz");
    if !archive.exists() {
        let temporary = artifacts.join("sqlmap.tar.gz");
        process::run(
            "curl",
            &[
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--max-time",
                "60",
                "--output",
                temporary.to_str().ok_or("non-UTF8 archive path")?,
                pins["sqlmap"]["archive"]
                    .as_str()
                    .ok_or("missing sqlmap archive pin")?,
            ],
        )
        .await?;
        if digest(&temporary)? != pins["sqlmap"]["sha256"] {
            return Err("sqlmap archive content does not match pins.json".into());
        }
        fs::copy(&temporary, &archive)?;
        fs::remove_file(temporary)?;
    }
    if digest(&archive)? != pins["sqlmap"]["sha256"] {
        return Err("sqlmap archive content does not match pins.json".into());
    }
    let source = artifacts.join("scanner-source");
    fs::create_dir(&source)?;
    process::run(
        "tar",
        &[
            "-xzf",
            archive.to_str().ok_or("non-UTF8 archive path")?,
            "-C",
            source.to_str().ok_or("non-UTF8 source path")?,
        ],
    )
    .await?;
    let children = fs::read_dir(&source)?.collect::<std::io::Result<Vec<_>>>()?;
    if children.len() != 1 {
        return Err("unexpected upstream archive layout".into());
    }
    let script = children[0].path().join("sqlmap.py");
    if !script.is_file() {
        return Err("missing upstream sqlmap.py".into());
    }
    Ok(script)
}

/// Every scan and fixture setting a profile declares, each of which reaches the scanner
/// invocation or the fixture. Parsing refuses a field outside this set, so a profile
/// cannot record a value the run would silently leave unapplied.
// [spec:pgorm:req:security.sqlmap.profiles+4]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub dbms: String,
    pub level: u64,
    pub risk: u64,
    pub concurrency: u64,
    pub retries: u64,
    pub time_sec: u64,
    pub postgres_statement_timeout_seconds: u64,
    pub postgres_lock_timeout_seconds: u64,
    pub http_timeout_seconds: u64,
    pub case_timeout_seconds: u64,
    /// The inclusive column-count range UNION tests try.
    pub union_columns: (u64, u64),
    /// Whether the scanner may retry a UNION test by fuzzing its column types.
    pub union_type_fuzzing: bool,
    pub verbosity: u64,
}

/// `cases` and `techniques` are read by `result::inventory`; `timeout_margin` is the prose
/// the timeouts are checked against.
const PROFILE_FIELDS: [&str; 16] = [
    "cases",
    "techniques",
    "timeout_margin",
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
];

impl Settings {
    pub fn from_profile(profile: &Value) -> std::result::Result<Self, String> {
        let fields = profile.as_object().ok_or("profile must be an object")?;
        if let Some(field) = fields
            .keys()
            .find(|f| !PROFILE_FIELDS.contains(&f.as_str()))
        {
            return Err(format!(
                "profile declares {field}, which the harness does not apply"
            ));
        }
        let number = |field: &str, least: u64, most: u64| {
            profile[field]
                .as_u64()
                .filter(|v| (least..=most).contains(v))
                .ok_or_else(|| {
                    format!("profile field {field} must be an integer from {least} to {most}")
                })
        };
        // sqlmap's own ceilings: level 5, risk 3, ten threads, verbosity 6. Below verbosity 1
        // the scanner drops the "does not seem to be injectable" warning that `interpret`
        // reads as evidence a parameter was tested, so every scan would end incomplete.
        let union_columns = match profile["union_columns"].as_array().map(Vec::as_slice) {
            Some([low, high]) => low
                .as_u64()
                .zip(high.as_u64())
                .filter(|(low, high)| (1..=*high).contains(low)),
            _ => None,
        }
        .ok_or("profile field union_columns must be [low, high] with 1 <= low <= high")?;
        let settings = Self {
            dbms: profile["dbms"]
                .as_str()
                .filter(|d| *d == "PostgreSQL")
                .ok_or(
                    "profile field dbms must be PostgreSQL, the only server the fixture provisions",
                )?
                .to_owned(),
            level: number("level", 1, 5)?,
            risk: number("risk", 1, 3)?,
            concurrency: number("concurrency", 1, 10)?,
            retries: number("retries", 0, u64::MAX)?,
            time_sec: number("time_sec", 1, u64::MAX)?,
            postgres_statement_timeout_seconds: number(
                "postgres_statement_timeout_seconds",
                1,
                u64::MAX,
            )?,
            postgres_lock_timeout_seconds: number("postgres_lock_timeout_seconds", 1, u64::MAX)?,
            http_timeout_seconds: number("http_timeout_seconds", 1, u64::MAX)?,
            case_timeout_seconds: number("case_timeout_seconds", 1, u64::MAX)?,
            union_columns,
            union_type_fuzzing: profile["union_type_fuzzing"]
                .as_bool()
                .ok_or("profile field union_type_fuzzing must be true or false")?,
            verbosity: number("verbosity", 1, 6)?,
        };
        if !profile["timeout_margin"]
            .as_str()
            .is_some_and(|m| !m.trim().is_empty())
        {
            return Err("profile must document its timeout margin".into());
        }
        // A timed control sleeps inside the statement, the statement ends before HTTP gives up
        // on it, and the outer deadline bounds the whole scan. The lock timeout stays outside
        // this chain, and may sit below the sleep: a timed control waits on no lock.
        if !(settings.time_sec < settings.postgres_statement_timeout_seconds
            && settings.postgres_statement_timeout_seconds < settings.http_timeout_seconds
            && settings.http_timeout_seconds < settings.case_timeout_seconds)
        {
            return Err("profile timeouts must nest: time_sec < postgres_statement_timeout_seconds < http_timeout_seconds < case_timeout_seconds".into());
        }
        Ok(settings)
    }
}

/// The GET parameter the manifest declares for a case: the one the route reads, sqlmap tests,
/// and the interpreter accepts findings for.
// [spec:pgorm:req:security.sqlmap.profiles+4]
fn field(case: &Value) -> Result<&str> {
    Ok(case["field"]
        .as_str()
        .filter(|f| !f.is_empty())
        .ok_or("missing test parameter")?)
}

/// Every manifest case's declared test parameter, keyed by case id: the table the adapter
/// routes requests by.
// [spec:pgorm:req:security.sqlmap.profiles+4]
pub fn fields(manifest: &Value) -> Result<BTreeMap<String, String>> {
    let mut fields = BTreeMap::new();
    for case in manifest["cases"]
        .as_array()
        .ok_or("missing manifest cases")?
    {
        let id = case["id"].as_str().ok_or("missing case id")?;
        if fields
            .insert(id.to_owned(), field(case)?.to_owned())
            .is_some()
        {
            return Err(format!("duplicated manifest case {id}").into());
        }
    }
    Ok(fields)
}

/// The value a request carries for a case's declared parameter. A request that omits it, or
/// repeats it, is refused rather than answered from some other parameter.
// [spec:pgorm:req:security.sqlmap.profiles+4]
pub fn declared_input<'a>(field: &str, query: &'a [(String, String)]) -> Result<&'a str> {
    let mut values = query
        .iter()
        .filter(|(name, _)| name == field)
        .map(|(_, value)| value.as_str());
    match (values.next(), values.next()) {
        (Some(value), None) => Ok(value),
        (None, _) => Err(format!("missing declared parameter {field}").into()),
        (Some(_), Some(_)) => Err(format!("repeated declared parameter {field}").into()),
    }
}

pub fn target(base: &str, case: &Value, mode: &str) -> Result<String> {
    let id = case["id"].as_str().ok_or("missing case id")?;
    let mut url = url::Url::parse(&format!("{base}/case/{mode}/{id}"))?;
    url.query_pairs_mut().append_pair(
        field(case)?,
        case["baseline"].as_str().ok_or("missing baseline")?,
    );
    Ok(url.into())
}

// [spec:pgorm:req:security.sqlmap.profiles+4]
pub fn scanner_args(
    python: &str,
    script: &Path,
    target: &str,
    technique: &str,
    settings: &Settings,
    case: &Value,
    output: &Path,
) -> Result<Vec<String>> {
    let mut args = vec![
        python.into(),
        script.to_string_lossy().into_owned(),
        "--url".into(),
        target.into(),
        "-p".into(),
        field(case)?.into(),
        "--dbms".into(),
        settings.dbms.clone(),
    ];
    args.extend(
        [
            "--batch",
            "--flush-session",
            "--fresh-queries",
            "--ignore-proxy",
            "--disable-coloring",
            "--technique",
            technique,
        ]
        .map(str::to_owned),
    );
    for (flag, value) in [
        ("--level", settings.level),
        ("--risk", settings.risk),
        ("--threads", settings.concurrency),
        ("--retries", settings.retries),
        ("--timeout", settings.http_timeout_seconds),
        ("--time-sec", settings.time_sec),
    ] {
        args.push(flag.into());
        args.push(value.to_string());
    }
    let (low, high) = settings.union_columns;
    args.extend([
        "--union-cols".into(),
        format!("{low}-{high}"),
        "--output-dir".into(),
    ]);
    args.push(output.join("session").to_string_lossy().into_owned());
    args.push("--report-json".into());
    args.push(output.join("scanner.json").to_string_lossy().into_owned());
    // "extending" and "include" both match the scanner's offer to run tests past the declared
    // level and risk; declining it is what holds the scan to the profile's values.
    args.push("--answers".into());
    args.push(format!(
        "extending=N,include=N,fuzzy={}",
        if settings.union_type_fuzzing {
            "Y"
        } else {
            "N"
        }
    ));
    args.push("-v".into());
    args.push(settings.verbosity.to_string());
    for option in ["prefix", "suffix"] {
        if let Some(value) = case[option].as_str() {
            args.push(format!("--{option}"));
            args.push(value.into());
        }
    }
    Ok(args)
}

async fn count(fixture: &Fixture, key: &str) -> Result<u64> {
    let (status, counts) = process::http(&format!("{}/counts", fixture.url)).await?;
    if status != 200 || !counts.is_object() {
        return Err("request accounting unavailable".into());
    }
    match counts.get(key) {
        Some(value) => value
            .as_u64()
            .ok_or_else(|| "malformed request counter".into()),
        None => Ok(0),
    }
}

struct Campaign<'a> {
    options: &'a Options,
    settings: Settings,
    script: PathBuf,
}

impl Campaign<'_> {
    async fn scan(
        &self,
        fixture: &Fixture,
        case: &Value,
        technique: &str,
        mode: &str,
    ) -> Result<ScanResult> {
        let started = Instant::now();
        let id = case["id"].as_str().ok_or("missing case id")?;
        let output = self
            .options
            .artifacts
            .join(format!("{id}-{technique}-{mode}"));
        fs::create_dir(&output)?;
        let target = target(&fixture.url, case, mode)?;
        let args = scanner_args(
            &self.options.python,
            &self.script,
            &target,
            technique,
            &self.settings,
            case,
            &output,
        )?;
        write_json(&output.join("command.json"), &args)?;
        // Keep the scanner invocation even when accounting is unavailable.
        let before = count(fixture, &format!("{mode}/{id}")).await;
        let invocation = async {
            let log = File::create(output.join("scanner.log"))?;
            let mut command = process::command(
                &args[0],
                &args[1..].iter().map(String::as_str).collect::<Vec<_>>(),
            );
            command.stdout(log.try_clone()?).stderr(log);
            let mut process = process::Process::spawn(&mut command)?;
            let status = process.wait(self.settings.case_timeout_seconds).await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(status.and_then(|s| {
                use std::os::unix::process::ExitStatusExt;
                s.code().or_else(|| s.signal().map(|s| -s))
            }))
        }
        .await;
        let after = count(fixture, &format!("{mode}/{id}")).await;
        let counts = match (before, after) {
            (Ok(before), Ok(after)) => after
                .checked_sub(before)
                .ok_or("request counter moved backwards".to_owned()),
            (Err(error), _) | (_, Err(error)) => Err(format!("request accounting failed: {error}")),
        };
        let report = fixture::scanner_report(&output);
        let log = fs::read_to_string(output.join("scanner.log")).unwrap_or_default();
        let mut result = interpret(
            &report,
            &log,
            invocation.as_ref().ok().copied().flatten(),
            &target,
            field(case)?,
            counts.as_ref().copied().unwrap_or(0),
        );
        if let Err(error) = invocation {
            result.complete = false;
            result
                .reason
                .push_str(&format!("; scanner invocation failed: {error}"));
        }
        if let Err(error) = counts {
            result.complete = false;
            result.reason.push_str(&format!("; {error}"));
        }
        result.seconds = Some((started.elapsed().as_secs_f64() * 1000.0).round() / 1000.0);
        write_json(&output.join("result.json"), &result)?;
        Ok(result)
    }

    async fn retained_scan(
        &self,
        fixture: &Fixture,
        case: &Value,
        technique: &str,
        mode: &str,
    ) -> ScanResult {
        match self.scan(fixture, case, technique, mode).await {
            Ok(result) => result,
            Err(error) => {
                let id = case["id"].as_str().unwrap_or("unknown");
                let output = self
                    .options
                    .artifacts
                    .join(format!("{id}-{technique}-{mode}"));
                let report = fixture::scanner_report(&output);
                let mut result = interpret(
                    &report,
                    "",
                    None,
                    &target(&fixture.url, case, mode).unwrap_or_default(),
                    field(case).unwrap_or_default(),
                    0,
                );
                result.reason = format!("scan infrastructure failure: {error}");
                let _ = write_json(&output.join("result.json"), &result);
                result
            }
        }
    }

    async fn pair(&self, fixture: &Fixture, case: &Value, technique: &str) -> Result<Value> {
        let id = case["id"].as_str().ok_or("missing case id")?;
        let mut baselines = BTreeMap::new();
        for mode in ["control", "protected"] {
            let baseline = match process::http(&target(&fixture.url, case, mode)?).await {
                Ok((status, body)) => json!({"status":status, "body":body}),
                Err(error) => json!({"status":null,"body":{},"error":error.to_string()}),
            };
            baselines.insert(mode, baseline);
        }
        let baseline_ok = baselines
            .values()
            .all(|b| b["status"] == 200 && b["body"]["invariant"] == true);
        if self.options.baseline_only {
            return Ok(
                json!({"outcome":"incomplete","reason":if baseline_ok {"baseline diagnostic"} else {"failed baseline"}, "baselines":baselines}),
            );
        }
        let control = self
            .retained_scan(fixture, case, technique, "control")
            .await;
        let mut protected = self
            .retained_scan(fixture, case, technique, "protected")
            .await;
        let invariant = match fixture::request_invariant(&self.options.artifacts, id) {
            Ok(invariant) => invariant,
            Err(error) => {
                protected.complete = false;
                protected
                    .reason
                    .push_str(&format!("; request evidence unreadable: {error}"));
                true
            }
        };
        let outcome = verdict(&control, &protected, technique, baseline_ok, invariant);
        let reason = match outcome {
            "vulnerable" => "protected finding or invariant violation".into(),
            "invalid-control" => format!(
                "control did not detect GET {} with technique {technique}",
                field(case)?
            ),
            "incomplete" if !baseline_ok => "failed baseline".into(),
            "incomplete" => format!(
                "control: {}; protected: {}",
                control.reason, protected.reason
            ),
            _ => "completed with detected control and clean protected route".into(),
        };
        Ok(
            json!({"outcome":outcome,"reason":reason,"api":case["api"],"control_detected":control.detects(technique),"control":control,"protected":protected,"baselines":baselines,"invariant":invariant}),
        )
    }

    /// Re-measures one declaration: the control alone, scanned with the exempted technique.
    // [spec:pgorm:req:security.sqlmap.profiles+4]
    async fn remeasure(&self, fixture: &Fixture, case: &Value, technique: &str) -> Result<Value> {
        let baseline = match process::http(&target(&fixture.url, case, "control")?).await {
            Ok((status, body)) => json!({"status":status, "body":body}),
            Err(error) => json!({"status":null,"body":{},"error":error.to_string()}),
        };
        let baseline_ok = baseline["status"] == 200 && baseline["body"]["invariant"] == true;
        let control = self
            .retained_scan(fixture, case, technique, "control")
            .await;
        let outcome = exemption_outcome(&control, technique, baseline_ok);
        let reason = match outcome {
            "falsified" => format!(
                "control detected GET {} with declared-inapplicable technique {technique}",
                field(case)?
            ),
            "incomplete" if !baseline_ok => "failed baseline".into(),
            "incomplete" => format!("control: {}", control.reason),
            _ => format!("completed control scan did not detect technique {technique}"),
        };
        Ok(
            json!({"outcome":outcome,"reason":reason,"api":case["api"],"detected":control.detects(technique),"control":control,"baselines":{"control":baseline}}),
        )
    }
}

fn summarize(report: &Value) -> String {
    let mut outcomes = BTreeMap::<String, usize>::new();
    let mut completed = 0;
    let mut attempted = 0;
    let mut findings = Vec::new();
    let mut undetected = Vec::new();
    let mut infrastructure = Vec::new();
    if let Some(results) = report["results"].as_object() {
        for (key, result) in results {
            *outcomes
                .entry(result["outcome"].as_str().unwrap_or("incomplete").into())
                .or_default() += 1;
            for mode in ["control", "protected"] {
                if result[mode].is_object() {
                    attempted += 1;
                    if result[mode]["complete"] == true {
                        completed += 1;
                    } else {
                        infrastructure.push(format!(
                            "{key}-{mode}: {}",
                            result[mode]["reason"].as_str().unwrap_or("missing result")
                        ));
                    }
                }
            }
            if result["protected"]["findings"]
                .as_array()
                .is_some_and(|f| !f.is_empty())
                || result["invariant"] == false
            {
                findings.push(key.clone());
            }
            if result["control_detected"] == false {
                undetected.push(key.clone());
            }
        }
    }
    // Exemptions ride beside the outcomes so a reader cannot mistake them for passes.
    let exempt: Vec<String> = report["inapplicable"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, entry)| {
            format!(
                "{key}: inapplicable ({}, CONTEXTS.md section {}) {}",
                entry["evidence"]["kind"].as_str().unwrap_or("?"),
                entry["evidence"]["contexts"].as_str().unwrap_or("?"),
                entry["reason"].as_str().unwrap_or("")
            )
        })
        .collect();
    format!(
        "{}{}{}: {}\nScans: {completed} complete, {attempted} attempted\nOutcomes: {outcomes:?}\nProtected findings: {}\nUndetected controls: {}\nInfrastructure failures: {}\n{}\nScheduled: {}; declared inapplicable: {}; falsified exemptions: {}\n{}\nCleanup: {}\nRun error: {}\nDirect regressions: {}\n",
        report["profile"].as_str().unwrap_or("unknown"),
        if report["subset"].as_array().is_some_and(|s| !s.is_empty()) {
            " subset"
        } else {
            ""
        },
        if report["remeasure"] == true {
            " exemption re-measurement"
        } else {
            ""
        },
        if report["pass"] == true {
            "PASS"
        } else {
            "FAIL"
        },
        findings.join(", "),
        undetected.join(", "),
        infrastructure.len(),
        infrastructure.join("\n"),
        report["results"]
            .as_object()
            .map_or(0, serde_json::Map::len),
        exempt.len(),
        report["falsified_exemptions"],
        exempt.join("\n"),
        report["cleanup_errors"],
        report["error"],
        report["direct_regressions"]
    )
}

async fn execute(
    options: &Options,
    root: &Path,
    here: &Path,
    report: &mut Value,
    fixture: &mut Fixture,
) -> Result<()> {
    let pins = read_json(&here.join("pins.json"))?;
    let manifest = read_json(&here.join("cases.json"))?;
    let profile = read_json(&here.join("profiles.json"))?[&options.profile].clone();
    let result::Inventory {
        mut work,
        mut exempt,
    } = inventory(&manifest, &profile, &options.subset)?;
    let settings = Settings::from_profile(&profile)?;
    let by_id: BTreeMap<_, _> = manifest["cases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["id"].as_str().map(|id| (id, c)))
        .collect();
    if options.remeasure {
        // The declarations are the work: each is measured on its own control, and stays
        // recorded under `inapplicable` as the claim that measurement tests.
        work = exempt
            .iter()
            .map(|(id, technique, _)| {
                Ok((
                    (*by_id.get(id.as_str()).ok_or("unknown case")?).clone(),
                    technique.clone(),
                ))
            })
            .collect::<Result<_>>()?;
        if work.is_empty() {
            return Err("the selected cases declare nothing inapplicable to re-measure".into());
        }
    }
    if options.baseline_only {
        // The diagnostic probes routes, not techniques, so it covers every selected
        // case including those left with no scheduled technique at all.
        let mut seen = BTreeSet::new();
        work.retain(|(c, _)| seen.insert(c["id"].as_str().unwrap_or_default().to_owned()));
        for (id, technique, _) in &exempt {
            if seen.insert(id.clone())
                && let Some(case) = by_id.get(id.as_str())
            {
                work.push(((*case).clone(), technique.clone()));
            }
        }
        exempt.clear();
    }
    report["inapplicable"] = json!(
        exempt
            .iter()
            .map(|(id, technique, entry)| (
                format!("{id}-{technique}"),
                json!({"technique":technique,"reason":entry["reason"],"evidence":entry["evidence"]})
            ))
            .collect::<BTreeMap<_, _>>()
    );
    let expected: Vec<_> = work
        .iter()
        .map(|(c, t)| format!("{}-{t}", c["id"].as_str().unwrap_or_default()))
        .collect();
    report["expected"] = json!(expected);
    let modes: &[&str] = if options.remeasure {
        &["control"]
    } else {
        &["control", "protected"]
    };
    report["expected_scans"] = json!(
        expected
            .iter()
            .flat_map(|key| modes.iter().map(move |mode| format!("{key}-{mode}")))
            .collect::<Vec<_>>()
    );
    report["results"] = json!(
        expected
            .iter()
            .map(|key| (
                key.clone(),
                json!({"outcome":"incomplete","reason":"not started"})
            ))
            .collect::<BTreeMap<_, _>>()
    );
    report["pins"] = pins.clone();
    report["source"] = source_identity(root).await?;
    report["manifest_sha256"] = digest(&here.join("cases.json"))?.into();
    report["profile_sha256"] = digest(&here.join("profiles.json"))?.into();
    report["effective_profile"] = profile.clone();
    write_json(&options.artifacts.join("report.json"), report)?;
    // Keep the manifests beside results so changing a checkout cannot change their meaning.
    for file in ["cases.json", "profiles.json", "pins.json"] {
        fs::copy(here.join(file), options.artifacts.join(file))?;
    }
    let release = python_release(
        &process::run(&options.python, &["--version"]).await?,
        pins["python"]
            .as_str()
            .ok_or("missing Python series in pins.json")?,
    )?;
    report["interpreter"] = json!({"executable":options.python,"version":release});
    let script = scanner(&pins, &root.join("target/sqlmap-cache"), &options.artifacts).await?;
    let manifest_path = here.join("adapter/Cargo.toml");
    process::run(
        "cargo",
        &[
            "build",
            "--locked",
            "--manifest-path",
            manifest_path.to_str().ok_or("non-UTF8 manifest path")?,
            "--bin",
            "pgorm-sqlmap-adapter",
        ],
    )
    .await?;
    let adapter = std::env::current_exe()?.with_file_name("pgorm-sqlmap-adapter");
    report["binaries"] = json!({"adapter_sha256":digest(&adapter)?,"harness_sha256":digest(&std::env::current_exe()?)?});
    fixture
        .start(
            &pins,
            &adapter,
            &settings,
            &options.artifacts.join("cases.json"),
        )
        .await?;
    report["database"] = fixture.settings.clone();
    let campaign = Campaign {
        options,
        settings,
        script,
    };
    for (case, technique) in work {
        let key = format!(
            "{}-{technique}",
            case["id"].as_str().ok_or("missing case id")?
        );
        println!("{key}: baseline");
        let measured = if options.remeasure {
            campaign.remeasure(fixture, &case, &technique).await
        } else {
            campaign.pair(fixture, &case, &technique).await
        };
        let result = match measured {
            Ok(result) => result,
            Err(error) => {
                json!({"outcome":"incomplete","reason":format!("case infrastructure failure: {error}")})
            }
        };
        println!(
            "{key}: {}",
            result["outcome"].as_str().unwrap_or("incomplete")
        );
        report["results"][&key] = result;
        write_json(&options.artifacts.join("report.json"), report)?;
    }
    if options.direct_regressions {
        println!("Running direct SQL security regressions");
        report["direct_regressions"] = match fixture.direct_regressions(root).await {
            Ok(result) => result,
            Err(error) => json!({"pass":false,"error":error.to_string()}),
        };
    }
    let final_source = source_identity(root).await?;
    report["source_unchanged"] = json!(report["source"] == final_source);
    report["source_after"] = final_source;
    if report["source_unchanged"] != true {
        return Err("source changed during campaign".into());
    }
    Ok(())
}

pub async fn run(options: Options) -> Result<bool> {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("missing harness directory")?;
    let root = here
        .parent()
        .and_then(Path::parent)
        .ok_or("missing repository root")?;
    if let Some(parent) = options.artifacts.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(&options.artifacts)?;
    let mut report = json!({"profile":options.profile,"subset":options.subset,"remeasure":options.remeasure,"results":{},"inapplicable":{},"falsified_exemptions":[],"cleanup_errors":[],"pass":false});
    write_json(&options.artifacts.join("report.json"), &report)?;
    let mut fixture = Fixture::new(&options.artifacts)?;
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let operation = tokio::select! {
        result = execute(&options, root, here, &mut report, &mut fixture) => result,
        _ = tokio::signal::ctrl_c() => Err("cancelled by SIGINT".into()),
        _ = term.recv() => Err("cancelled by SIGTERM".into()),
    };
    if let Err(error) = operation {
        report["error"] = error.to_string().into();
        eprintln!("{error}");
    }
    let cleanup = fixture.close().await;
    let expected: Vec<String> =
        serde_json::from_value(report["expected"].clone()).unwrap_or_default();
    let results: BTreeMap<String, Value> =
        serde_json::from_value(report["results"].clone()).unwrap_or_default();
    let inapplicable: BTreeMap<String, Value> =
        serde_json::from_value(report["inapplicable"].clone()).unwrap_or_default();
    let falsified_exemptions = falsified(&results, &inapplicable);
    report["cleanup_errors"] = json!(cleanup);
    report["falsified_exemptions"] = json!(falsified_exemptions);
    let accounted = if options.remeasure {
        remeasured
    } else {
        aggregate
    };
    report["pass"] = json!(
        !options.baseline_only
            && report.get("error").is_none()
            && accounted(&expected, &results, &cleanup, &falsified_exemptions)
            && (!options.direct_regressions || report["direct_regressions"]["pass"] == true)
    );
    write_json(&options.artifacts.join("report.json"), &report)?;
    let summary = summarize(&report);
    fs::write(options.artifacts.join("summary.txt"), &summary)?;
    println!("{summary}Artifacts: {}", options.artifacts.display());
    Ok(report["pass"] == true)
}
