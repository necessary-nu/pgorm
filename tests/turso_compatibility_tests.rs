//! The same operations run on PostgreSQL, isolated native memory and a fresh file.
#![cfg(feature = "turso")]

use crate::turso_compatibility;

use futures::TryStreamExt;
use pgorm::{Connection, DecodeRaw};
use pgorm_driver::{Client, NoTls, Row, error::SqlState, types::Type};
use pgorm_query::Values;
use std::sync::Arc;
use tokio::{sync::Barrier, task::JoinSet, time::timeout};
use turso_compatibility::{Backend, Fixture, LIMIT, Result};

#[derive(Debug, PartialEq)]
struct Scalar {
    boolean: Option<bool>,
    small: Option<i16>,
    integer: Option<i32>,
    big: Option<i64>,
    text: Option<String>,
    bytes: Option<Vec<u8>>,
    float_bits: Option<u64>,
}

impl Scalar {
    fn read(row: &Row) -> Result<Self> {
        Ok(Self {
            boolean: row.try_get(0)?,
            small: row.try_get(1)?,
            integer: row.try_get(2)?,
            big: row.try_get(3)?,
            text: row.try_get(4)?,
            bytes: row.try_get(5)?,
            float_bits: row.try_get::<_, Option<f64>>(6)?.map(f64::to_bits),
        })
    }
}

async fn scalar_snapshot(_: Backend, client: &mut Client) -> Result<Vec<Scalar>> {
    client.batch_execute("CREATE TABLE scalar(id integer PRIMARY KEY, b boolean, s smallint, i integer, n bigint, t text, bytes bytea, f double precision)").await?;
    let types = [
        Type::INT4,
        Type::BOOL,
        Type::INT2,
        Type::INT4,
        Type::INT8,
        Type::TEXT,
        Type::BYTEA,
        Type::FLOAT8,
    ];
    let insert = client
        .prepare_typed("INSERT INTO scalar VALUES($1,$2,$3,$4,$5,$6,$7,$8)", &types)
        .await?;
    assert_eq!(insert.params(), types);
    assert_eq!(
        client
            .execute(
                &insert,
                &[
                    &1_i32,
                    &true,
                    &i16::MIN,
                    &i32::MAX,
                    &i64::MIN,
                    &"λ 🦀",
                    &vec![0_u8, 255],
                    &f64::from_bits(0x3ff0000000000001)
                ]
            )
            .await?,
        1
    );
    assert_eq!(
        client
            .execute(
                &insert,
                &[
                    &2_i32,
                    &None::<bool>,
                    &None::<i16>,
                    &None::<i32>,
                    &None::<i64>,
                    &None::<&str>,
                    &None::<&[u8]>,
                    &None::<f64>
                ]
            )
            .await?,
        1
    );
    assert_eq!(
        client
            .execute(
                &insert,
                &[
                    &3_i32,
                    &false,
                    &0_i16,
                    &0_i32,
                    &0_i64,
                    &"",
                    &Vec::<u8>::new(),
                    &0.0_f64
                ]
            )
            .await?,
        1
    );
    let projection = "SELECT b,s,i,n,t,bytes,f FROM scalar";
    for suffix in [" WHERE false", " WHERE id=2", " ORDER BY id"] {
        let statement = client.prepare(&format!("{projection}{suffix}")).await?;
        let actual: Vec<_> = statement
            .columns()
            .iter()
            .map(|column| column.type_().clone())
            .collect();
        assert_eq!(actual, types[1..]);
        assert_eq!(
            statement
                .columns()
                .iter()
                .map(|column| column.name())
                .collect::<Vec<_>>(),
            ["b", "s", "i", "n", "t", "bytes", "f"]
        );
    }
    let rows = client
        .query(&format!("{projection} ORDER BY id"), &[])
        .await?;
    rows.iter().map(Scalar::read).collect()
}

// [spec:pgorm:req:turso.compatibility/test]
#[tokio::test]
async fn scalar_values_and_metadata_match_postgres() -> Result {
    let fixture = Fixture::new("turso_scalar_oracle").await?;
    let outcomes = fixture.clients(scalar_snapshot).await;
    fixture.finish().await?;
    let outcomes = outcomes?;
    let expected = &outcomes[0].1;
    assert_eq!(expected.len(), 3);
    assert_eq!(expected[0].big, Some(i64::MIN));
    assert_eq!(expected[1].text, None);
    assert_eq!(expected[2].text.as_deref(), Some(""));
    for (backend, actual) in &outcomes[1..] {
        assert_eq!(actual, expected, "{backend:?}");
    }
    Ok(())
}

async fn execution_snapshot(_: Backend, client: &mut Client) -> Result<(Vec<u64>, Vec<i32>)> {
    let mut counts = vec![
        client
            .execute("CREATE TABLE counts(id integer PRIMARY KEY)", &[])
            .await?,
    ];
    let insert = client
        .prepare_typed("INSERT INTO counts VALUES($1)", &[Type::INT4])
        .await?;
    for id in 1..=3_i32 {
        counts.push(client.execute(&insert, &[&id]).await?);
    }
    counts.push(client.execute("SELECT id FROM counts", &[]).await?);
    counts.push(
        client
            .execute_typed(
                "UPDATE counts SET id=id+$1 WHERE id<=2 RETURNING id",
                &[(&10_i32, Type::INT4)],
            )
            .await?,
    );
    counts.push(
        client
            .execute("DELETE FROM counts WHERE id=3 RETURNING id", &[])
            .await?,
    );
    let mut rows = Box::pin(
        client
            .query_raw(
                "INSERT INTO counts VALUES(20),(21) RETURNING id",
                std::iter::empty::<i32>(),
            )
            .await?,
    );
    assert_eq!(rows.rows_affected(), None);
    let mut returned = Vec::new();
    while let Some(row) = rows.try_next().await? {
        returned.push(row.try_get(0)?);
    }
    counts.push(rows.rows_affected().ok_or("missing completed row count")?);
    Ok((counts, returned))
}

// [spec:pgorm:req:turso.compatibility/test]
#[tokio::test]
async fn counts_and_statement_reuse_match_postgres() -> Result {
    let fixture = Fixture::new("turso_counts_oracle").await?;
    let outcomes = fixture.clients(execution_snapshot).await;
    fixture.finish().await?;
    for (backend, (counts, rows)) in outcomes? {
        assert_eq!(counts, [0, 1, 1, 1, 3, 2, 1, 2], "{backend:?}");
        assert_eq!(rows, [20, 21], "{backend:?}");
    }
    Ok(())
}

async fn transaction_snapshot(_: Backend, client: &mut Client) -> Result<(Vec<String>, Vec<i32>)> {
    client
        .batch_execute("CREATE TABLE transactions(id integer PRIMARY KEY)")
        .await?;
    let mut transaction = client.transaction().await?;
    transaction
        .execute("INSERT INTO transactions VALUES(1)", &[])
        .await?;
    let savepoint = transaction.transaction().await?;
    savepoint
        .execute("INSERT INTO transactions VALUES(2)", &[])
        .await?;
    savepoint.rollback().await?;
    let savepoint = transaction.transaction().await?;
    let duplicate = savepoint
        .execute("INSERT INTO transactions VALUES(1)", &[])
        .await
        .expect_err("duplicate must fail");
    let aborted = savepoint
        .query_one("SELECT 1", &[])
        .await
        .expect_err("savepoint must be aborted");
    let codes = [&duplicate, &aborted]
        .into_iter()
        .map(|error| {
            error
                .code()
                .map(|code| code.code().to_owned())
                .ok_or_else(|| "native error lost SQLSTATE".into())
        })
        .collect::<Result<Vec<_>>>()?;
    savepoint.rollback().await?;
    transaction
        .execute("INSERT INTO transactions VALUES(3)", &[])
        .await?;
    transaction.commit().await?;
    let transaction = client.transaction().await?;
    transaction
        .execute("INSERT INTO transactions VALUES(4)", &[])
        .await?;
    transaction.rollback().await?;
    let rows = client
        .query("SELECT id FROM transactions ORDER BY id", &[])
        .await?
        .iter()
        .map(|row| row.try_get(0))
        .collect::<std::result::Result<_, _>>()?;
    Ok((codes, rows))
}

// [spec:pgorm:req:turso.compatibility/test]
#[tokio::test]
async fn savepoints_and_error_states_match_postgres() -> Result {
    let fixture = Fixture::new("turso_transactions_oracle").await?;
    let outcomes = fixture.clients(transaction_snapshot).await;
    fixture.finish().await?;
    for (backend, (codes, rows)) in outcomes? {
        assert_eq!(codes, ["23505", "25P02"], "{backend:?}");
        assert_eq!(rows, [1, 3], "{backend:?}");
    }
    Ok(())
}

const MANY: &str = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000000) SELECT x::bigint FROM n";

async fn stream_snapshot(_: Backend, client: &mut Client) -> Result<(i64, i32)> {
    let finite = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<1000) SELECT x::bigint FROM n";
    let mut rows = Box::pin(client.query_raw(finite, std::iter::empty::<i32>()).await?);
    let retained = rows.try_next().await?.ok_or("missing first row")?;
    drop(rows);
    timeout(LIMIT, client.check_connection()).await??;
    let mut rows = Box::pin(client.query_raw(MANY, std::iter::empty::<i32>()).await?);
    assert_eq!(
        rows.try_next()
            .await?
            .ok_or("missing first row")?
            .try_get::<_, i64>(0)?,
        1
    );
    client.cancel_token().cancel_query(NoTls).await?;
    let error = timeout(LIMIT, async {
        loop {
            match rows.try_next().await {
                Ok(Some(_)) => (),
                Ok(None) => return Err("cancelled stream ended successfully".into()),
                Err(error) => return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(error),
            }
        }
    })
    .await??;
    assert_eq!(error.code(), Some(&SqlState::QUERY_CANCELED));
    assert_eq!(rows.rows_affected(), None);
    drop(rows);
    let next = timeout(LIMIT, client.query_one("SELECT 42", &[])).await??;
    Ok((retained.try_get(0)?, next.try_get(0)?))
}

// [spec:pgorm:req:turso.compatibility/test]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streaming_cancellation_and_reuse_match_postgres() -> Result {
    let fixture = Fixture::new("turso_stream_oracle").await?;
    let outcomes = fixture.clients(stream_snapshot).await;
    fixture.finish().await?;
    for (backend, result) in outcomes? {
        assert_eq!(result, (1, 42), "{backend:?}");
    }
    Ok(())
}

// [spec:pgorm:req:turso.compatibility/test]
#[tokio::test]
async fn metadata_mismatches_remain_explicit() -> Result {
    let fixture = Fixture::new("turso_metadata_oracle").await?;
    let outcomes = fixture
        .clients(async |backend, client| {
            client
                .batch_execute(
                    "CREATE TABLE erased(s smallserial, b bigserial, v varchar, n numeric, r real)",
                )
                .await?;
            let statement = client.prepare("SELECT * FROM erased WHERE false").await?;
            let expected = match backend {
                Backend::Postgres => [
                    Type::INT2,
                    Type::INT8,
                    Type::VARCHAR,
                    Type::NUMERIC,
                    Type::FLOAT4,
                ],
                Backend::Memory | Backend::File => [
                    Type::INT4,
                    Type::INT4,
                    Type::TEXT,
                    Type::FLOAT8,
                    Type::FLOAT8,
                ],
            };
            assert_eq!(
                statement
                    .columns()
                    .iter()
                    .map(|column| column.type_().clone())
                    .collect::<Vec<_>>(),
                expected,
                "{backend:?}"
            );
            for column in statement.columns() {
                assert_eq!(column.table_oid().is_some(), backend == Backend::Postgres);
                assert_eq!(column.column_id().is_some(), backend == Backend::Postgres);
            }
            assert!(client.query(&statement, &[]).await?.is_empty());
            Ok(())
        })
        .await;
    fixture.finish().await?;
    outcomes?;
    Ok(())
}

async fn concurrent_borrowers(pool: &pgorm::DatabasePool) -> Result {
    let setup = pool.get().await?;
    setup.batch_execute("CREATE TABLE pool_values(id integer PRIMARY KEY); INSERT INTO pool_values VALUES(1),(2),(3),(4)").await?;
    drop(setup);
    let ready = Arc::new(Barrier::new(4));
    let mut workers = JoinSet::new();
    for id in 1..=4_i32 {
        let pool = pool.clone();
        let ready = Arc::clone(&ready);
        workers.spawn(async move {
            // Each holds a distinct connection at this gate; a serialized pool would deadlock.
            let first = pool.get().await?;
            ready.wait().await;
            drop(first);
            for _ in 0..16 {
                let db = pool.get().await?;
                let actual = (
                    "SELECT id FROM pool_values WHERE id=$1",
                    Values(vec![id.into()]),
                )
                    .into_tuple::<i32>()
                    .one(&db)
                    .await?;
                assert_eq!(actual, id);
                let mut rows = Box::pin(
                    db.query_raw(
                        "SELECT id FROM pool_values ORDER BY id",
                        std::iter::empty::<i32>(),
                    )
                    .await?,
                );
                let mut count = 0;
                while rows.try_next().await?.is_some() {
                    count += 1;
                }
                assert_eq!(count, 4);
            }
            Ok::<_, pgorm::Error>(())
        });
    }
    timeout(LIMIT, async {
        while let Some(result) = workers.join_next().await {
            result??;
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    })
    .await??;
    assert_eq!(pool.status().size, 4);
    let db = pool.get().await?;
    assert_eq!(
        db.query_one("SELECT count(*) FROM pool_values", &[])
            .await?
            .try_get::<_, i64>(0)?,
        4
    );
    Ok(())
}

// [spec:pgorm:req:turso.compatibility/test]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_public_pools_match_postgres() -> Result {
    let fixture = Fixture::new("turso_pool_oracle").await?;
    let outcome: Result = async {
        for backend in [Backend::Postgres, Backend::Memory, Backend::File] {
            let pool = fixture.pool(backend).await?;
            let outcome = concurrent_borrowers(&pool).await;
            pool.close();
            drop(pool);
            outcome?;
        }
        Ok(())
    }
    .await;
    let cleanup = fixture.finish().await;
    outcome.and(cleanup)
}
