mod support;
use support as report;

use futures::TryStreamExt;
use pgorm::{ConnectionTrait, DatabasePool, TransactionTrait, entity::prelude::*};
use pgorm_driver::{Client, NoTls, types::ToSql};
use report::{command, hash_file, source_fingerprint};
use serde_json::{Value, json};
use std::{
    error::Error,
    hint::black_box,
    path::PathBuf,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::Barrier, task::JoinSet};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const SCALAR: &str = "SELECT $1::BIGINT";
const LOOKUP: &str = "SELECT id, payload, value FROM bench_row WHERE id = $1";
const UPDATE: &str = "UPDATE bench_row SET payload = $2, value = $3::BIGINT + $4::BIGINT + $5::BIGINT + $6::BIGINT + $7::BIGINT + $8::BIGINT + $9::BIGINT + $10::BIGINT + $11::BIGINT + $12::BIGINT + $13::BIGINT + $14::BIGINT + $15::BIGINT + $16::BIGINT WHERE id = $1";

mod model {
    use pgorm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "bench_row")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i64,
        pub payload: String,
        pub value: i64,
    }
    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

struct Options {
    samples: usize,
    warmup: usize,
    rounds: usize,
    workers: usize,
    pool_size: usize,
    rows: usize,
    source_root: PathBuf,
    output: Option<PathBuf>,
    raw_samples: bool,
}
impl Options {
    fn read() -> Result<Self> {
        let mut options = Self {
            samples: 1_000,
            warmup: 100,
            rounds: 3,
            workers: 4,
            pool_size: 4,
            rows: 128,
            source_root: std::env::current_dir()?,
            output: None,
            raw_samples: false,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--bench" {
                continue;
            }
            let value = args.next().ok_or_else(|| format!("{arg} needs a value"))?;
            match arg.as_str() {
                "--samples" => options.samples = value.parse()?,
                "--warmup" => options.warmup = value.parse()?,
                "--rounds" => options.rounds = value.parse()?,
                "--workers" => options.workers = value.parse()?,
                "--pool-size" => options.pool_size = value.parse()?,
                "--rows" => options.rows = value.parse()?,
                "--source-root" => options.source_root = value.into(),
                "--output" => options.output = Some(value.into()),
                "--raw-samples" => options.raw_samples = value.parse()?,
                _ => return Err(format!("unknown argument {arg}").into()),
            }
        }
        if options.samples == 0
            || options.warmup == 0
            || options.rounds == 0
            || options.workers == 0
            || options.pool_size == 0
        {
            return Err("samples, warmup, rounds, workers and pool-size must be positive".into());
        }
        if !(1..=1_024).contains(&options.rows) {
            return Err("rows must be in 1..=1024".into());
        }
        Ok(options)
    }
}

fn summary(
    name: &str,
    round: usize,
    elapsed: std::time::Duration,
    mut times: Vec<u64>,
    raw_samples: bool,
) -> Value {
    times.sort_unstable();
    let count = times.len();
    let quantile = |p: usize| times[((count * p).div_ceil(100) - 1).min(count - 1)];
    let mut report = json!({"name": name, "round": round, "samples": count, "elapsed_ns": elapsed.as_nanos(), "operations_per_second": count as f64 / elapsed.as_secs_f64(), "p50_ns": quantile(50), "p95_ns": quantile(95), "p99_ns": quantile(99), "min_ns": times[0], "max_ns": times[count - 1]});
    if raw_samples {
        report["sorted_latency_ns"] = json!(times);
    }
    report
}

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
            return Err(format!("{name} returned the wrong value during warmup").into());
        }
    }
    let mut times = Vec::with_capacity(options.samples);
    let elapsed = Instant::now();
    for _ in 0..options.samples {
        let start = Instant::now();
        let actual = black_box(operation().await?);
        times.push(start.elapsed().as_nanos().try_into()?);
        if actual != expected {
            return Err(format!("{name} returned the wrong value").into());
        }
    }
    let result = summary(name, round, elapsed.elapsed(), times, options.raw_samples);
    eprintln!(
        "round {round}: {name}: p50={}ns p95={}ns ops/s={:.0}",
        result["p50_ns"],
        result["p95_ns"],
        result["operations_per_second"].as_f64().unwrap()
    );
    Ok(result)
}

fn parameters_sql() -> String {
    format!(
        "SELECT {}",
        (1..=16)
            .map(|i| format!("${i}::BIGINT"))
            .collect::<Vec<_>>()
            .join(" + ")
    )
}

async fn direct(driver: &mut Client, round: usize, options: &Options) -> Result<Vec<Value>> {
    let mut results = Vec::new();
    let scalar = driver.prepare(SCALAR).await?;
    results.push(
        measure("driver/scalar", round, options, 17, async || {
            Ok(driver.query_one(&scalar, &[&17i64]).await?.get::<_, i64>(0) as u64)
        })
        .await?,
    );
    let params_stmt = driver.prepare(&parameters_sql()).await?;
    let values = [17i64; 16];
    let params: Vec<&(dyn ToSql + Sync)> = values.iter().map(|v| v as _).collect();
    results.push(
        measure("driver/params16", round, options, 272, async || {
            Ok(driver
                .query_one(&params_stmt, &params)
                .await?
                .get::<_, i64>(0) as u64)
        })
        .await?,
    );
    let model_stmt = driver.prepare(LOOKUP).await?;
    results.push(
        measure("driver/model_lookup", round, options, 17, async || {
            let row = driver.query_one(&model_stmt, &[&17i64]).await?;
            let model = model::Model {
                id: row.try_get(0)?,
                payload: row.try_get(1)?,
                value: row.try_get(2)?,
            };
            Ok(black_box(model).value as u64)
        })
        .await?,
    );
    let write_stmt = driver.prepare(UPDATE).await?;
    let payload = "y".repeat(32);
    let mut write_values = [1i64; 14];
    write_values[0] = 4;
    let mut write_params: Vec<&(dyn ToSql + Sync)> = vec![&17i64, &payload];
    write_params.extend(write_values.iter().map(|v| v as &(dyn ToSql + Sync)));
    results.push(
        measure("driver/update16", round, options, 1, async || {
            Ok(driver.execute(&write_stmt, &write_params).await?)
        })
        .await?,
    );
    let stream_stmt = driver
        .prepare(&format!(
            "SELECT id, payload, value FROM bench_row ORDER BY id LIMIT {}",
            options.rows
        ))
        .await?;
    let sum = (options.rows * (options.rows + 1) / 2) as u64;
    results.push(
        measure("driver/stream", round, options, sum, async || {
            let stream = driver
                .query_raw(&stream_stmt, std::iter::empty::<&i64>())
                .await?;
            futures::pin_mut!(stream);
            let mut sum = 0;
            while let Some(row) = stream.try_next().await? {
                sum += row.try_get::<_, i64>(0)? as u64;
                black_box(row.try_get::<_, String>(1)?);
                black_box(row.try_get::<_, i64>(2)?);
            }
            Ok(sum)
        })
        .await?,
    );
    results.push(
        measure("driver/transaction", round, options, 17, async || {
            let tx = driver.transaction().await?;
            let value: i64 = tx.query_one(&scalar, &[&17i64]).await?.get(0);
            tx.commit().await?;
            Ok(value as u64)
        })
        .await?,
    );
    Ok(results)
}

async fn orm(pool: &DatabasePool, round: usize, options: &Options) -> Result<Vec<Value>> {
    let mut orm = pool.get().await?;
    let mut results = Vec::new();
    results.push(
        measure("orm/scalar", round, options, 17, async || {
            Ok(orm.query_one(SCALAR, &[&17i64]).await?.get::<_, i64>(0) as u64)
        })
        .await?,
    );
    let params_sql = parameters_sql();
    let values = [17i64; 16];
    let params: Vec<&(dyn ToSql + Sync)> = values.iter().map(|v| v as _).collect();
    results.push(
        measure("orm/params16", round, options, 272, async || {
            Ok(orm.query_one(&params_sql, &params).await?.get::<_, i64>(0) as u64)
        })
        .await?,
    );
    results.push(
        measure("orm/model_lookup", round, options, 17, async || {
            let model = model::Entity::find_by_id(17i64).one(&orm).await?;
            Ok(black_box(model).value as u64)
        })
        .await?,
    );
    let payload = "y".repeat(32);
    let mut write_values = [1i64; 14];
    write_values[0] = 4;
    let mut write_params: Vec<&(dyn ToSql + Sync)> = vec![&17i64, &payload];
    write_params.extend(write_values.iter().map(|v| v as &(dyn ToSql + Sync)));
    results.push(
        measure("orm/update16", round, options, 1, async || {
            Ok(orm.execute(UPDATE, &write_params).await?)
        })
        .await?,
    );
    let sum = (options.rows * (options.rows + 1) / 2) as u64;
    results.push(
        measure("orm/stream", round, options, sum, async || {
            let stream = model::Entity::find()
                .order_by_asc(model::Column::Id)
                .limit(options.rows as u64)
                .stream(&orm)
                .await?;
            futures::pin_mut!(stream);
            let mut sum = 0;
            while let Some(row) = stream.try_next().await? {
                sum += black_box(row).id as u64;
            }
            Ok(sum)
        })
        .await?,
    );
    results.push(
        measure("orm/transaction", round, options, 17, async || {
            Ok(orm
                .transaction(async |tx| {
                    Ok::<_, pgorm::Error>(
                        tx.query_one(SCALAR, &[&17i64]).await?.get::<_, i64>(0) as u64
                    )
                })
                .await?)
        })
        .await?,
    );
    Ok(results)
}

async fn pooled(
    driver_pool: &pgorm_pool::Pool,
    orm_pool: &DatabasePool,
    is_orm: bool,
    round: usize,
    options: &Options,
) -> Result<Value> {
    let samples = options.samples;
    let warmup = options.warmup;
    let ready = Arc::new(Barrier::new(options.workers + 1));
    let start_gate = Arc::new(Barrier::new(options.workers + 1));
    let mut tasks = JoinSet::new();
    for _ in 0..options.workers {
        let driver = driver_pool.clone();
        let orm = orm_pool.clone();
        let ready = ready.clone();
        let start_gate = start_gate.clone();
        tasks.spawn(async move {
            let mut times = Vec::<u64>::with_capacity(samples);
            if is_orm {
                for _ in 0..warmup {
                    let db = orm.get().await?;
                    black_box(db.query_one(SCALAR, &[&17i64]).await?.get::<_, i64>(0));
                }
                ready.wait().await;
                start_gate.wait().await;
                for _ in 0..samples {
                    let start = Instant::now();
                    let db = orm.get().await?;
                    let value = db.query_one(SCALAR, &[&17i64]).await?.get::<_, i64>(0);
                    drop(db);
                    times.push(start.elapsed().as_nanos().try_into()?);
                    if black_box(value) != 17 {
                        return Err("pooled query returned the wrong value".into());
                    }
                }
            } else {
                for _ in 0..warmup {
                    let db = driver.get().await?;
                    let stmt = db.prepare_cached(SCALAR).await?;
                    black_box(db.query_one(&stmt, &[&17i64]).await?.get::<_, i64>(0));
                }
                ready.wait().await;
                start_gate.wait().await;
                for _ in 0..samples {
                    let start = Instant::now();
                    let db = driver.get().await?;
                    let stmt = db.prepare_cached(SCALAR).await?;
                    let value = db.query_one(&stmt, &[&17i64]).await?.get::<_, i64>(0);
                    drop(stmt);
                    drop(db);
                    times.push(start.elapsed().as_nanos().try_into()?);
                    if black_box(value) != 17 {
                        return Err("pooled query returned the wrong value".into());
                    }
                }
            }
            Ok::<_, Box<dyn Error + Send + Sync>>(times)
        });
    }
    tokio::select! {
        _ = ready.wait() => {},
        result = tasks.join_next() => {
            result.ok_or("a pool worker ended before timing began")???;
            return Err("a pool worker finished without entering the timing barrier".into());
        },
    }
    let elapsed = Instant::now();
    start_gate.wait().await;
    let mut times = Vec::with_capacity(samples * options.workers);
    while let Some(result) = tasks.join_next().await {
        times.extend(result??);
    }
    Ok(summary(
        if is_orm {
            "orm/concurrent_pool"
        } else {
            "driver/concurrent_pool"
        },
        round,
        elapsed.elapsed(),
        times,
        options.raw_samples,
    ))
}

async fn run(base: &str, database: &str, options: &Options) -> Result<Value> {
    let mut config = pgorm_test_support::config(base, database);
    config.application_name("pgorm-driver-baseline");
    let transport = match config.get_hosts().first() {
        Some(pgorm_driver::config::Host::Tcp(_)) => "tcp",
        #[cfg(unix)]
        Some(pgorm_driver::config::Host::Unix(_)) => "unix-socket",
        _ => "unspecified",
    };
    let (mut driver, connection) = config.connect(NoTls).await?;
    let connection = tokio::spawn(connection);
    let version: String = driver.query_one("SELECT version()", &[]).await?.get(0);
    let settings = driver.query("SELECT name, setting FROM pg_settings WHERE name IN ('fsync','full_page_writes','synchronous_commit','shared_buffers','wal_level','max_connections') ORDER BY name", &[]).await?.into_iter().map(|row| (row.get::<_, String>(0), Value::String(row.get(1)))).collect::<serde_json::Map<_,_>>();
    let active_other: i64 = driver.query_one("SELECT count(*) FROM pg_stat_activity WHERE datname IS NOT NULL AND application_name <> 'pgorm-driver-baseline' AND state = 'active'", &[]).await?.get(0);
    driver.batch_execute("CREATE TABLE bench_row(id bigint PRIMARY KEY, payload text NOT NULL, value bigint NOT NULL); INSERT INTO bench_row SELECT i, repeat('x', 32), i FROM generate_series(1, 1024) i; ANALYZE bench_row;").await?;
    let orm_pool = pgorm::connect_with_builder(config.clone(), |builder| {
        builder.max_size(options.pool_size)
    })?;
    let driver_pool = pgorm_pool::Pool::builder(pgorm_pool::Manager::new(config, NoTls))
        .max_size(options.pool_size)
        .build()?;
    let mut results = Vec::new();
    for round in 0..options.rounds {
        if round % 2 == 0 {
            results.extend(direct(&mut driver, round, options).await?);
            results.extend(orm(&orm_pool, round, options).await?);
        } else {
            results.extend(orm(&orm_pool, round, options).await?);
            results.extend(direct(&mut driver, round, options).await?);
        }
        for is_orm in if round % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            results.push(pooled(&driver_pool, &orm_pool, is_orm, round, options).await?);
        }
    }
    drop(orm_pool);
    drop(driver_pool);
    drop(driver);
    connection.await??;
    Ok(json!({
        "schema_version": 1, "backend": "postgresql", "timestamp_unix_seconds": SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        "server_version": version, "server_settings": settings, "other_active_server_sessions_at_start": active_other,
        "os": std::env::consts::OS, "arch": std::env::consts::ARCH, "available_parallelism": std::thread::available_parallelism()?.get(),
        "kernel": command(&options.source_root,"uname", &["-srm"]), "cpu": command(&options.source_root,"sysctl", &["-n", "machdep.cpu.brand_string"]),
        "available_rustc_at_run": command(&options.source_root,"rustc", &["--version", "--verbose"]), "git_head_at_run_end": command(&options.source_root,"git", &["rev-parse", "HEAD"]),
        "git_status_at_run_end": command(&options.source_root,"git", &["status", "--short"]),
        "source_fingerprint_at_run_end": source_fingerprint(&options.source_root)?, "driver_provenance_at_run": std::fs::read_to_string(options.source_root.join("crates/pgorm-driver/UPSTREAM.md"))?,
        "profile": "bench", "rustflags_at_run": std::env::var("RUSTFLAGS").ok(), "samples_per_worker": options.samples, "warmup_per_worker": options.warmup,
        "rounds": options.rounds, "workers": options.workers, "pool_size": options.pool_size, "stream_rows": options.rows,
        "transport": transport, "tokio_worker_threads": 4,
        "raw_samples_included": options.raw_samples,
        "cargo_features": { "turso": cfg!(feature = "turso"), "macros": cfg!(feature = "macros"), "with-json": cfg!(feature = "with-json"), "with-jiff": cfg!(feature = "with-jiff"), "with-uuid": cfg!(feature = "with-uuid"), "postgres-array": cfg!(feature = "postgres-array"), "tests-cfg": cfg!(feature = "tests-cfg"), "metrics-fingerprint": cfg!(feature = "metrics-fingerprint") },
        "latency_clock": "std::time::Instant", "allocation_measurements": null,
        "notes": ["Each run owns a scoped PostgreSQL database; setup, preparation and warmup are excluded from per-operation latency.", "Driver model lookup/streaming decodes equivalent fields; ORM paths also build queries and decode derived models.", "Concurrent latency includes pool acquisition, cached preparation, query, decoding and release; scalar paths retain one connection.", "Latency recording is uninstrumented for allocation counts; do not infer allocation improvements from this report.", "Compiler, flags, source tree and git state are inspected at run time, not embedded compilation provenance. The executable hash identifies the measured binary; preserve its build command and source snapshot for reproduction."],
        "measurements": results,
    }))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    if cfg!(debug_assertions) {
        return Err("run this benchmark with the release profile".into());
    }
    let options = Options::read()?;
    let source_at_start = source_fingerprint(&options.source_root)?;
    let executable = hash_file(&options.source_root, &std::env::current_exe()?)?;
    dotenvy::from_path(options.source_root.join(".env.local")).ok();
    dotenvy::from_path(options.source_root.join(".env")).ok();
    let base = std::env::var("DATABASE_URL")?;
    let database = pgorm_test_support::scoped_name("driver_baseline");
    pgorm_test_support::create_database(&base, &database).await?;
    let result = run(&base, &database, &options).await;
    let cleanup = pgorm_test_support::drop_database(&base, &database).await;
    let mut report = match (result, cleanup) {
        (Ok(report), Ok(())) => report,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(cleanup)) => {
            return Err(format!("benchmark database cleanup failed: {cleanup}").into());
        }
        (Err(error), Err(cleanup)) => {
            return Err(format!(
                "benchmark failed: {error}; database cleanup also failed: {cleanup}"
            )
            .into());
        }
    };
    report["source_tree_changed_during_run"] =
        json!(report["source_fingerprint_at_run_end"].as_str() != Some(&source_at_start));
    report["source_fingerprint_at_run_start"] = json!(source_at_start);
    report["executable_git_blob_hash"] = json!(executable);
    let report = serde_json::to_string_pretty(&report)?;
    if let Some(path) = &options.output {
        std::fs::write(path, report)?;
    } else {
        println!("{report}");
    }
    Ok(())
}
