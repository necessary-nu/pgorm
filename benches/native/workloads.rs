// [spec:pgorm:req:turso.performance]
use crate::{Options, Result, Scratch, allocation, measure, summary};
use futures::TryStreamExt;
use pgorm::{
    ConnectionTrait, DatabasePool, DecodeRaw, TransactionTrait, Update, UpdateMany,
    entity::prelude::*,
    pgorm_query::{ExprTrait, Values},
};
use pgorm_driver::{
    Client, Config, NoTls,
    types::{ToSql, Type},
};
use pgorm_pool::{Manager, ManagerConfig, Pool};
use serde_json::{Value as Json, json};
use std::{
    hint::black_box,
    num::NonZeroUsize,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::Barrier, task::JoinSet};
use turso_core::Value;
use turso_pg::{PgConnection, Statement};

const SCALAR: &str = "SELECT $1::BIGINT";
const PAYLOAD: &str = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
const OTHER_PAYLOAD: &str = "yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy";
const SCHEMA: &str = "CREATE TABLE bench_row(id BIGINT PRIMARY KEY, payload TEXT NOT NULL, value BIGINT NOT NULL); WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<1024) INSERT INTO bench_row SELECT x,'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx',x FROM n";

mod model {
    use pgorm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "bench_row")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i64,
        pub payload: String,
        pub value: i64,
    }
    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

fn params_sql() -> String {
    format!(
        "SELECT ({})::BIGINT",
        (1..=16)
            .map(|index| format!("${index}::BIGINT"))
            .collect::<Vec<_>>()
            .join(" + ")
    )
}

fn update(payload: &str) -> UpdateMany<model::Entity> {
    let mut sum = Expr::value(4_i64);
    for _ in 0..13 {
        sum = sum.add(1_i64);
    }
    Update::many(model::Entity)
        .col_expr(model::Column::Payload, Expr::value(payload))
        .col_expr(model::Column::Value, sum)
        .filter(model::Column::Id.eq(17_i64))
}

fn lookup_sql() -> String {
    model::Entity::find_by_id(17_i64).limit(1).build().0
}

fn stream_sql(rows: usize) -> String {
    model::Entity::find()
        .order_by_asc(model::Column::Id)
        .limit(rows as u64)
        .build()
        .0
}

fn memory_sql(rows: usize) -> String {
    format!(
        "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<{rows}) SELECT x::BIGINT AS id, '{PAYLOAD}'::TEXT AS payload, x::BIGINT AS value FROM n"
    )
}

fn open(path: &str) -> Result<(Arc<turso_core::Database>, PgConnection)> {
    let (_, database) = turso_pg::open_database(
        path,
        None,
        turso_pg::OpenFlags::default(),
        turso_pg::tursopg_database_opts(),
    )?;
    let connection = PgConnection::connect(&database)?;
    Ok((database, connection))
}

fn bind(statement: &mut Statement, values: impl IntoIterator<Item = Value>) -> Result<()> {
    statement.clear_bindings();
    for (index, value) in values.into_iter().enumerate() {
        statement.bind_at(
            NonZeroUsize::new(index + 1).expect("one-based parameter index"),
            value,
        )?;
    }
    Ok(())
}

fn scalar(statement: &mut Statement, values: impl IntoIterator<Item = Value>) -> Result<u64> {
    bind(statement, values)?;
    let mut found = None;
    let mut count = 0;
    let result = statement.run_with_row_callback(|row| {
        found = Some(row.get::<i64>(0)? as u64);
        count += 1;
        Ok(())
    });
    let reset = statement.reset();
    result?;
    reset?;
    if count != 1 {
        return Err(format!("scalar returned {count} rows").into());
    }
    found.ok_or_else(|| "missing scalar row".into())
}

fn native_model(row: &turso_core::Row) -> turso_pg::Result<model::Model> {
    Ok(model::Model {
        id: row.get(0)?,
        payload: row.get(1)?,
        value: row.get(2)?,
    })
}

fn driver_model(row: &pgorm_driver::Row) -> Result<model::Model> {
    Ok(model::Model {
        id: row.try_get(0)?,
        payload: row.try_get(1)?,
        value: row.try_get(2)?,
    })
}

macro_rules! measured {
    ($results:ident, $layer:literal, $name:literal, $round:expr, $options:expr, $expected:expr, $body:expr) => {
        if $options.includes($name) {
            $results.push(
                measure(
                    concat!($layer, "/", $name),
                    $round,
                    $options,
                    $expected,
                    $body,
                )
                .await?,
            );
        }
    };
}

async fn direct(connection: &PgConnection, round: usize, options: &Options) -> Result<Vec<Json>> {
    let mut results = Vec::new();
    let mut scalar_stmt = connection.prepare(SCALAR)?;
    measured!(
        results,
        "direct",
        "scalar",
        round,
        options,
        17,
        async || { scalar(&mut scalar_stmt, [Value::from_i64(17)]) }
    );
    let mut params = connection.prepare(params_sql())?;
    measured!(
        results,
        "direct",
        "params16",
        round,
        options,
        272,
        async || { scalar(&mut params, (0..16).map(|_| Value::from_i64(17))) }
    );
    let mut lookup = connection.prepare(lookup_sql())?;
    measured!(
        results,
        "direct",
        "model_lookup",
        round,
        options,
        17,
        async || {
            bind(&mut lookup, [Value::from_i64(17), Value::from_i64(1)])?;
            let mut value = None;
            let mut rows = 0;
            lookup.run_with_row_callback(|row| {
                value = Some(black_box(native_model(row)?).value as u64);
                rows += 1;
                Ok(())
            })?;
            lookup.reset()?;
            if rows != 1 {
                return Err("model lookup must return one row".into());
            }
            value.ok_or_else(|| "missing model".into())
        }
    );
    let mut write = connection.prepare(update(PAYLOAD).build().0)?;
    let mut turn = false;
    measured!(
        results,
        "direct",
        "update16",
        round,
        options,
        1,
        async || {
            turn = !turn;
            bind(
                &mut write,
                std::iter::once(Value::build_text(
                    if turn { OTHER_PAYLOAD } else { PAYLOAD }.to_owned(),
                ))
                .chain(std::iter::once(Value::from_i64(4)))
                .chain((0..13).map(|_| Value::from_i64(1)))
                .chain(std::iter::once(Value::from_i64(17))),
            )?;
            write.run_ignore_rows()?;
            let count = write.n_change() as u64;
            write.reset()?;
            Ok(count)
        }
    );
    let mut stream = connection.prepare(stream_sql(options.rows))?;
    let expected = (options.rows * (options.rows + 1) / 2) as u64;
    measured!(
        results,
        "direct",
        "stream",
        round,
        options,
        expected,
        async || {
            bind(&mut stream, [Value::from_i64(options.rows as i64)])?;
            let mut sum = 0;
            stream.run_with_row_callback(|row| {
                sum += black_box(native_model(row)?).id as u64;
                Ok(())
            })?;
            stream.reset()?;
            Ok(sum)
        }
    );
    measured!(
        results,
        "direct",
        "transaction",
        round,
        options,
        17,
        async || {
            connection.execute("BEGIN")?;
            let value = scalar(&mut scalar_stmt, [Value::from_i64(17)])?;
            connection.execute("COMMIT")?;
            Ok(value)
        }
    );
    Ok(results)
}

async fn driver(client: &mut Client, round: usize, options: &Options) -> Result<Vec<Json>> {
    let mut results = Vec::new();
    let scalar = client.prepare_typed(SCALAR, &[Type::INT8]).await?;
    measured!(
        results,
        "driver",
        "scalar",
        round,
        options,
        17,
        async || {
            Ok(client
                .query_one(&scalar, &[&17_i64])
                .await?
                .get::<_, i64>(0) as u64)
        }
    );
    let params = client
        .prepare_typed(&params_sql(), &[Type::INT8; 16])
        .await?;
    let values = [17_i64; 16];
    let bound: Vec<&(dyn ToSql + Sync)> = values.iter().map(|value| value as _).collect();
    measured!(
        results,
        "driver",
        "params16",
        round,
        options,
        272,
        async || { Ok(client.query_one(&params, &bound).await?.get::<_, i64>(0) as u64) }
    );
    let lookup = client
        .prepare_typed(&lookup_sql(), &[Type::INT8; 2])
        .await?;
    measured!(
        results,
        "driver",
        "model_lookup",
        round,
        options,
        17,
        async || {
            let row = client.query_one(&lookup, &[&17_i64, &1_i64]).await?;
            Ok(black_box(driver_model(&row)?).value as u64)
        }
    );
    let mut types = vec![Type::INT8; 16];
    types[0] = Type::TEXT;
    let write = client
        .prepare_typed(&update(PAYLOAD).build().0, &types)
        .await?;
    let ones = [1_i64; 13];
    let mut turn = false;
    let first = OTHER_PAYLOAD.to_owned();
    let second = PAYLOAD.to_owned();
    let mut write_values: Vec<&(dyn ToSql + Sync)> = vec![&first, &4_i64];
    write_values.extend(ones.iter().map(|value| value as &(dyn ToSql + Sync)));
    write_values.push(&17_i64);
    measured!(
        results,
        "driver",
        "update16",
        round,
        options,
        1,
        async || {
            turn = !turn;
            write_values[0] = if turn { &first } else { &second };
            Ok(client.execute(&write, &write_values).await?)
        }
    );
    let stream = client
        .prepare_typed(&stream_sql(options.rows), &[Type::INT8])
        .await?;
    let stream_limit = options.rows as i64;
    let expected = (options.rows * (options.rows + 1) / 2) as u64;
    measured!(
        results,
        "driver",
        "stream",
        round,
        options,
        expected,
        async || {
            let rows = client
                .query_raw(&stream, std::iter::once(&stream_limit))
                .await?;
            futures::pin_mut!(rows);
            let mut sum = 0;
            while let Some(row) = rows.try_next().await? {
                sum += black_box(driver_model(&row)?).id as u64;
            }
            Ok(sum)
        }
    );
    measured!(
        results,
        "driver",
        "transaction",
        round,
        options,
        17,
        async || {
            let tx = client.transaction().await?;
            let value = tx.query_one(&scalar, &[&17_i64]).await?.get::<_, i64>(0);
            tx.commit().await?;
            Ok(value as u64)
        }
    );
    Ok(results)
}

async fn orm(pool: &DatabasePool, round: usize, options: &Options) -> Result<Vec<Json>> {
    let mut db = pool.get().await?;
    let mut results = Vec::new();
    measured!(results, "orm", "scalar", round, options, 17, async || {
        Ok((SCALAR, Values(vec![17_i64.into()]))
            .into_tuple::<i64>()
            .one(&db)
            .await? as u64)
    });
    let params = params_sql();
    measured!(
        results,
        "orm",
        "params16",
        round,
        options,
        272,
        async || {
            Ok((params.as_str(), Values(vec![17_i64.into(); 16]))
                .into_tuple::<i64>()
                .one(&db)
                .await? as u64)
        }
    );
    measured!(
        results,
        "orm",
        "model_lookup",
        round,
        options,
        17,
        async || { Ok(black_box(model::Entity::find_by_id(17_i64).one(&db).await?).value as u64) }
    );
    let mut turn = false;
    measured!(results, "orm", "update16", round, options, 1, async || {
        turn = !turn;
        Ok(update(if turn { OTHER_PAYLOAD } else { PAYLOAD })
            .exec(&db)
            .await?)
    });
    let expected = (options.rows * (options.rows + 1) / 2) as u64;
    measured!(
        results,
        "orm",
        "stream",
        round,
        options,
        expected,
        async || {
            let rows = model::Entity::find()
                .order_by_asc(model::Column::Id)
                .limit(options.rows as u64)
                .stream(&db)
                .await?;
            futures::pin_mut!(rows);
            let mut sum = 0;
            while let Some(row) = rows.try_next().await? {
                sum += black_box(row).id as u64;
            }
            Ok(sum)
        }
    );
    measured!(
        results,
        "orm",
        "transaction",
        round,
        options,
        17,
        async || {
            Ok(db
                .transaction(async |tx| {
                    Ok::<_, pgorm::Error>(
                        (SCALAR, Values(vec![17_i64.into()]))
                            .into_tuple::<i64>()
                            .one(tx)
                            .await? as u64,
                    )
                })
                .await?)
        }
    );
    Ok(results)
}

async fn pooled(
    driver: &Pool,
    orm: &DatabasePool,
    is_orm: bool,
    round: usize,
    options: &Options,
) -> Result<Json> {
    let ready = Arc::new(Barrier::new(options.workers + 1));
    let start = Arc::new(Barrier::new(options.workers + 1));
    let mut tasks = JoinSet::new();
    for _ in 0..options.workers {
        let driver = driver.clone();
        let orm = orm.clone();
        let ready = Arc::clone(&ready);
        let start = Arc::clone(&start);
        let warmup = options.warmup;
        let samples = options.samples;
        tasks.spawn(async move {
            let mut times = Vec::<u64>::with_capacity(samples);
            for iteration in 0..warmup + samples {
                if iteration == warmup {
                    ready.wait().await;
                    start.wait().await;
                }
                let begin = Instant::now();
                let value = if is_orm {
                    let db = orm.get().await?;
                    (SCALAR, Values(vec![17_i64.into()]))
                        .into_tuple::<i64>()
                        .one(&db)
                        .await?
                } else {
                    let db = driver.get().await?;
                    let statement = db.prepare_typed_cached(SCALAR, &[Type::INT8]).await?;
                    db.query_one(&statement, &[&17_i64]).await?.get::<_, i64>(0)
                };
                if iteration >= warmup {
                    times.push(begin.elapsed().as_nanos().try_into()?);
                }
                if black_box(value) != 17 {
                    return Err("pooled scalar returned wrong value".into());
                }
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(times)
        });
    }
    tokio::select! {
        _ = ready.wait() => {},
        result = tasks.join_next() => {
            result.ok_or("pool worker ended before warmup barrier")???;
            return Err("pool worker skipped warmup barrier".into());
        }
    }
    let mut times = Vec::with_capacity(options.samples * options.workers);
    let allocations = allocation::Snapshot::start();
    let begin = Instant::now();
    start.wait().await;
    while let Some(result) = tasks.join_next().await {
        times.extend(result??);
    }
    let elapsed = begin.elapsed();
    let allocations = allocations.finish();
    Ok(summary(
        if is_orm {
            "orm/concurrent_pool"
        } else {
            "driver/concurrent_pool"
        },
        round,
        elapsed,
        times,
        allocations,
    ))
}

fn direct_concurrent(
    database: &Arc<turso_core::Database>,
    round: usize,
    options: &Options,
) -> Result<Json> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let mut starts = Vec::new();
    let mut threads = Vec::new();
    for _ in 0..options.workers {
        let database = Arc::clone(database);
        let ready = ready_tx.clone();
        let done = done_tx.clone();
        let (start_tx, start_rx) = std::sync::mpsc::channel();
        starts.push(start_tx);
        let warmup = options.warmup;
        let samples = options.samples;
        threads.push(std::thread::spawn(move || -> Result<()> {
            let prepared = (|| -> Result<_> {
                let connection = PgConnection::connect(&database)?;
                let mut statement = connection.prepare(SCALAR)?;
                for _ in 0..warmup {
                    if scalar(&mut statement, [Value::from_i64(17)])? != 17 {
                        return Err("direct warmup returned wrong value".into());
                    }
                }
                Ok((connection, statement))
            })();
            let mut times = Vec::with_capacity(samples);
            ready.send(prepared.as_ref().map(|_| ()).map_err(ToString::to_string))?;
            let (connection, mut statement) = prepared?;
            if !start_rx.recv()? {
                return Err("another direct worker failed".into());
            }
            let measured = (|| -> Result<Vec<u64>> {
                for _ in 0..samples {
                    let begin = Instant::now();
                    let value = scalar(&mut statement, [Value::from_i64(17)])?;
                    times.push(begin.elapsed().as_nanos().try_into()?);
                    if black_box(value) != 17 {
                        return Err("direct scalar returned wrong value".into());
                    }
                }
                Ok(times)
            })();
            done.send(measured)?;
            start_rx.recv()?;
            drop(statement);
            connection.close()?;
            Ok(())
        }));
    }
    drop(ready_tx);
    drop(done_tx);
    for _ in 0..options.workers {
        let ready = ready_rx.recv()?;
        if let Err(error) = ready {
            for start in starts {
                let _ = start.send(false);
            }
            for thread in threads {
                let _ = thread.join();
            }
            return Err(error.into());
        }
    }
    let mut times = Vec::with_capacity(options.samples * options.workers);
    let allocations = allocation::Snapshot::start();
    let begin = Instant::now();
    for start in &starts {
        start.send(true)?;
    }
    let mut failed = None;
    for _ in 0..options.workers {
        match done_rx.recv()? {
            Ok(samples) => times.extend(samples),
            Err(error) => failed = Some(error),
        }
    }
    let elapsed = begin.elapsed();
    let allocations = allocations.finish();
    for start in starts {
        start.send(false)?;
    }
    for thread in threads {
        thread.join().map_err(|_| "direct worker panicked")??;
    }
    if let Some(error) = failed {
        return Err(error);
    }
    Ok(summary(
        "direct/concurrent_connections",
        round,
        elapsed,
        times,
        allocations,
    ))
}

async fn memory(
    connection: &PgConnection,
    client: &Client,
    pool: &DatabasePool,
    options: &Options,
) -> Result<Vec<Json>> {
    let mut results = Vec::new();
    let db = pool.get().await?;
    for rows in [1_000_usize, 100_000, 1_000_000] {
        let sql = memory_sql(rows);
        let expected = (rows * (rows + 1) / 2) as u64;
        if options.layer == "all" || options.layer == "direct" {
            let mut statement = connection.prepare(&sql)?;
            let allocations = allocation::Snapshot::start();
            let mut sum = 0;
            statement.run_with_row_callback(|row| {
                sum += black_box(native_model(row)?).id as u64;
                Ok(())
            })?;
            statement.reset()?;
            let allocations = allocations.finish();
            if sum != expected {
                return Err("direct memory stream returned wrong rows".into());
            }
            results
                .push(json!({"layer":"direct","rows":rows,"allocation_measurements":allocations}));
        }
        if options.layer == "all" || options.layer == "driver" {
            let statement = client.prepare(&sql).await?;
            let allocations = allocation::Snapshot::start();
            let stream = client
                .query_raw(&statement, std::iter::empty::<&i64>())
                .await?;
            futures::pin_mut!(stream);
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut sum = 0;
            while let Some(row) = stream.try_next().await? {
                sum += black_box(driver_model(&row)?).id as u64;
            }
            let allocations = allocations.finish();
            if sum != expected {
                return Err("driver memory stream returned wrong rows".into());
            }
            results.push(json!({"layer":"driver","rows":rows,"initial_consumer_pause_ms":50,"allocation_measurements":allocations}));
        }
        if options.layer == "all" || options.layer == "orm" {
            // Prime the exact cached statement before measuring its stream.
            let warm = db
                .query_raw(sql.as_str(), std::iter::empty::<&i64>())
                .await?;
            drop(warm);
            db.query_one("SELECT 1", &[]).await?;
            let allocations = allocation::Snapshot::start();
            let stream = (sql.as_str(), Values(Vec::new()))
                .into_model::<model::Model>()
                .stream(&db)
                .await?;
            futures::pin_mut!(stream);
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut sum = 0;
            while let Some(row) = stream.try_next().await? {
                sum += black_box(row).id as u64;
            }
            let allocations = allocations.finish();
            if sum != expected {
                return Err("ORM memory stream returned wrong rows".into());
            }
            results.push(json!({"layer":"orm","rows":rows,"initial_consumer_pause_ms":50,"allocation_measurements":allocations}));
        }
    }
    Ok(results)
}

pub(crate) async fn run(options: &Options, scratch: &Scratch) -> Result<Json> {
    let (database, native) = open(&scratch.database(options, "direct"))?;
    native.execute(SCHEMA)?;
    let settings = json!({
        "sync_mode": format!("{:?}", native.inner().get_sync_mode()),
        "cache_size": native.inner().get_cache_size(),
        "page_size": format!("{:?}", native.inner().get_page_size()),
        "mvcc": database.mvcc_enabled(),
        "experimental_mvcc_passive_checkpoint": database.experimental_mvcc_passive_checkpoint_enabled(),
        "source": "public getters on the direct native database/connection; all layers use identical open_database/tursopg_database_opts and PgConnection::connect defaults",
        "io_backend_from_pinned_source": if options.database == "memory" { "MemoryIO" } else { "PlatformIO" },
        "io_backend_source": "turso core/database.rs::Database::io_for_path at the recorded native_revision; open_database receives no custom VFS",
        "checkpoint_thresholds": null,
        "postgresql_durability_equivalence": null,
        "durability_note": "SyncMode is sampled from the engine, not a verification of power-loss guarantees. Checkpoint thresholds are not measured; PostgreSQL-equivalent durability is not established.",
    });
    let driver_config: Config = scratch.database(options, "driver").parse()?;
    let (mut client, lifecycle) = driver_config.connect(NoTls).await?;
    let lifecycle = tokio::spawn(lifecycle);
    client.batch_execute(SCHEMA).await?;
    let driver_pool = Pool::builder(Manager::from_config(
        driver_config,
        NoTls,
        ManagerConfig::default(),
    ))
    .max_size(options.pool_size)
    .build()?;
    let orm_pool = pgorm::connect_with(
        scratch.database(options, "orm").parse()?,
        NoTls,
        ManagerConfig::default(),
        |builder| builder.max_size(options.pool_size),
    )?;
    orm_pool.get().await?.batch_execute(SCHEMA).await?;
    // Acquire every pool slot together so connections exist and are typed-cache warm.
    let mut held_driver = Vec::new();
    let mut held_orm = Vec::new();
    for _ in 0..options.pool_size {
        let driver = driver_pool.get().await?;
        let statement = driver.prepare_typed_cached(SCALAR, &[Type::INT8]).await?;
        driver.query_one(&statement, &[&17_i64]).await?;
        held_driver.push(driver);
        let orm = orm_pool.get().await?;
        (SCALAR, Values(vec![17_i64.into()]))
            .into_tuple::<i64>()
            .one(&orm)
            .await?;
        held_orm.push(orm);
    }
    drop(held_driver);
    drop(held_orm);
    let mut measurements = Vec::new();
    if !options.stream_memory {
        for round in 0..options.rounds {
            measurements.push(
                measure("harness/control", round, options, 17, async || {
                    Ok(black_box(17))
                })
                .await?,
            );
            for offset in 0..3 {
                let layer = ["direct", "driver", "orm"][(round + offset) % 3];
                if options.layer != "all" && options.layer != layer {
                    continue;
                }
                match layer {
                    "direct" => measurements.extend(direct(&native, round, options).await?),
                    "driver" => measurements.extend(driver(&mut client, round, options).await?),
                    "orm" => measurements.extend(orm(&orm_pool, round, options).await?),
                    _ => unreachable!(),
                }
                if options.includes("concurrent_pool") {
                    measurements.push(match layer {
                        "direct" => direct_concurrent(&database, round, options)?,
                        "driver" => pooled(&driver_pool, &orm_pool, false, round, options).await?,
                        "orm" => pooled(&driver_pool, &orm_pool, true, round, options).await?,
                        _ => unreachable!(),
                    });
                }
            }
        }
    }
    let memory = if options.stream_memory {
        memory(&native, &client, &orm_pool, options).await?
    } else {
        Vec::new()
    };
    driver_pool.close();
    orm_pool.close();
    drop(driver_pool);
    drop(orm_pool);
    drop(client);
    lifecycle.await??;
    native.close()?;
    Ok(json!({
        "schema_version": 2, "backend": "native-turso", "native_revision": "adf3b41f99ebb6b1cdd1b60ae08b483bc45686fa",
        "database": options.database, "native_settings": settings, "layer": options.layer, "workload": options.workload,
        "profile": "release", "allocation_profile": cfg!(feature = "alloc-profile"), "tokio_worker_threads": 4,
        "samples_per_worker": options.samples, "warmup_per_worker": options.warmup, "rounds": options.rounds,
        "workers": options.workers, "pool_size": options.pool_size, "stream_rows": options.rows,
        "copy_measurements": null, "rss_measurements": null,
        "sql": {"scalar": SCALAR, "params16": params_sql(), "lookup": lookup_sql(), "update16": update(PAYLOAD).build().0, "stream": stream_sql(options.rows)},
        "notes": [
            "Latency and allocation builds are separate; profiling timings must not establish performance rankings.",
            "Setup, connections, preparation and warmup are outside per-operation timing; all layers decode the same owned model fields.",
            "Native update16 alternates equal-length payloads and binds16 values; PostgreSQL historical workload remains unchanged.",
            "Direct concurrency is a lower bound with one retained connection per OS thread, without pool acquisition. Driver/ORM include acquisition and Fast recycling.",
            "Global profiling counts requested Rust heap storage including worker threads and measurement coordination, excluding C allocator activity, allocator fragmentation, mappings, stacks, OS caches, RSS and byte copies.",
            "Allocation sample storage is preallocated; JSON/report construction is outside counter snapshots. Realloc counts both old deallocated and new allocated requested sizes.",
            "File and memory modes have different durability. Compare adapter layers within one mode; no cross-engine durability equivalence is claimed.",
            "Runtime-inspected compiler/source/flags are not compiled provenance; preserve executable hashes and build records.",
        ],
        "measurements": measurements, "stream_memory_measurements": memory,
    }))
}
