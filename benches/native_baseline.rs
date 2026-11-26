//! Matched native workloads; allocation instrumentation is a separate build.

mod native;
mod support;

use native::{allocation, workloads};
use support as report;

use report::{command, hash_file, source_fingerprint};
use serde_json::{Value, json};
use std::{
    error::Error,
    hint::black_box,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

struct Options {
    samples: usize,
    warmup: usize,
    rounds: usize,
    workers: usize,
    pool_size: usize,
    rows: usize,
    database: String,
    layer: String,
    workload: String,
    source_root: PathBuf,
    output: PathBuf,
    stream_memory: bool,
}

impl Options {
    fn read() -> Result<Self> {
        let mut options = Self {
            samples: 5_000,
            warmup: 500,
            rounds: 3,
            workers: 4,
            pool_size: 4,
            rows: 128,
            database: "memory".into(),
            layer: "all".into(),
            workload: "all".into(),
            source_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
            output: PathBuf::from("native-baseline.json"),
            stream_memory: false,
        };
        let mut args = std::env::args().skip(1);
        while let Some(name) = args.next() {
            let value = args.next().ok_or_else(|| format!("{name} needs a value"))?;
            match name.as_str() {
                "--samples" => options.samples = value.parse()?,
                "--warmup" => options.warmup = value.parse()?,
                "--rounds" => options.rounds = value.parse()?,
                "--workers" => options.workers = value.parse()?,
                "--pool-size" => options.pool_size = value.parse()?,
                "--rows" => options.rows = value.parse()?,
                "--database" => options.database = value,
                "--layer" => options.layer = value,
                "--workload" => options.workload = value,
                "--source-root" => options.source_root = value.into(),
                "--output" => options.output = value.into(),
                "--stream-memory" => options.stream_memory = value.parse()?,
                _ => return Err(format!("unknown argument {name}").into()),
            }
        }
        if [
            options.samples,
            options.warmup,
            options.rounds,
            options.workers,
            options.pool_size,
        ]
        .contains(&0)
        {
            return Err("samples, warmup, rounds, workers and pool-size must be positive".into());
        }
        if !(1..=1_024).contains(&options.rows) {
            return Err("rows must be in 1..=1024".into());
        }
        if !["memory", "file"].contains(&options.database.as_str())
            || !["all", "direct", "driver", "orm"].contains(&options.layer.as_str())
            || ![
                "all",
                "scalar",
                "params16",
                "model_lookup",
                "update16",
                "stream",
                "transaction",
                "concurrent_pool",
            ]
            .contains(&options.workload.as_str())
        {
            return Err("unknown database, layer or workload".into());
        }
        if options.stream_memory && !cfg!(feature = "alloc-profile") {
            return Err("stream-memory requires the alloc-profile build".into());
        }
        options.source_root = options.source_root.canonicalize()?;
        Ok(options)
    }

    fn includes(&self, workload: &str) -> bool {
        self.workload == "all" || self.workload == workload
    }
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "pgorm-native-performance-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn database(&self, options: &Options, layer: &str) -> String {
        if options.database == "memory" {
            ":memory:".into()
        } else {
            self.0
                .join(format!("{layer}.db"))
                .to_string_lossy()
                .into_owned()
        }
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("benchmark cleanup failed for {}: {error}", self.0.display());
        }
    }
}

fn summary(
    name: &str,
    round: usize,
    elapsed: Duration,
    mut times: Vec<u64>,
    allocations: Value,
) -> Value {
    times.sort_unstable();
    let count = times.len();
    let quantile = |p: usize| times[((count * p).div_ceil(100) - 1).min(count - 1)];
    json!({
        "name": name, "round": round, "samples": count,
        "elapsed_ns": elapsed.as_nanos(), "operations_per_second": count as f64 / elapsed.as_secs_f64(),
        "p50_ns": quantile(50), "p95_ns": quantile(95), "p99_ns": quantile(99),
        "min_ns": times[0], "max_ns": times[count-1],
        "allocation_measurements": allocations,
    })
}

// [spec:pgorm:req:turso.performance/test] reject incomplete or incorrect measured operations
async fn measure<F>(
    name: &str,
    round: usize,
    options: &Options,
    expected: u64,
    mut operation: F,
) -> Result<Value>
where
    F: AsyncFnMut() -> Result<u64>,
{
    for _ in 0..options.warmup {
        if black_box(operation().await?) != expected {
            return Err(format!("{name} warmup returned the wrong value").into());
        }
    }
    let mut times = Vec::with_capacity(options.samples);
    let allocations = allocation::Snapshot::start();
    let elapsed = Instant::now();
    for _ in 0..options.samples {
        let start = Instant::now();
        let actual = black_box(operation().await?);
        times.push(start.elapsed().as_nanos().try_into()?);
        if actual != expected {
            return Err(format!("{name} returned {actual}, expected {expected}").into());
        }
    }
    let elapsed = elapsed.elapsed();
    let allocations = allocations.finish();
    let report = summary(name, round, elapsed, times, allocations);
    eprintln!(
        "round {round}: {name}: p50={}ns p95={}ns",
        report["p50_ns"], report["p95_ns"]
    );
    Ok(report)
}

// [spec:pgorm:req:turso.performance] identical native workloads with separate profiling builds
fn main() -> Result<()> {
    if cfg!(debug_assertions) {
        return Err("run the benchmark with --release".into());
    }
    let options = Options::read()?;
    let scratch = Scratch::new()?;
    let source_before = source_fingerprint(&options.source_root)?;
    let executable = hash_file(&options.source_root, &std::env::current_exe()?)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;
    let result = runtime.block_on(workloads::run(&options, &scratch));
    drop(runtime);
    let mut report = result?;
    let source_after = source_fingerprint(&options.source_root)?;
    report["source_tree_changed_during_run"] = json!(source_before != source_after);
    report["source_fingerprint_at_run_start"] = json!(source_before);
    report["source_fingerprint_at_run_end"] = json!(source_after);
    report["executable_git_blob_hash"] = json!(executable);
    report["native_lockfile_git_blob_hash"] = json!(hash_file(
        &options.source_root,
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.lock")
    )?);
    report["timestamp_unix_seconds"] =
        json!(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs());
    report["available_rustc_at_run"] = json!(command(
        &options.source_root,
        "rustc",
        &["--version", "--verbose"]
    ));
    report["git_head_at_run_end"] =
        json!(command(&options.source_root, "git", &["rev-parse", "HEAD"]));
    report["git_status_at_run_end"] =
        json!(command(&options.source_root, "git", &["status", "--short"]));
    report["kernel"] = json!(command(&options.source_root, "uname", &["-srm"]));
    report["cpu"] = json!(command(
        &options.source_root,
        "sysctl",
        &["-n", "machdep.cpu.brand_string"]
    ));
    report["rustflags_at_run"] = json!(std::env::var("RUSTFLAGS").ok());
    report["available_parallelism"] = json!(std::thread::available_parallelism()?.get());
    std::fs::write(&options.output, serde_json::to_string_pretty(&report)?)?;
    Ok(())
}
