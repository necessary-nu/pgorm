#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, bakery_chain::*, setup::*};
use futures::future::join_all;
use pgorm::entity::prelude::*;
use pretty_assertions::assert_eq;

const CONNECTIONS: usize = 4;

async fn insert_bakery<C>(db: &C, name: &str, profit_margin: f64) -> Result<(), Error>
where
    C: ConnectionMut,
{
    bakery::ActiveModel {
        name: set(name),
        profit_margin: set(profit_margin),
        ..Default::default()
    }
    .insert(db)
    .await?;

    Ok(())
}

async fn count_bakeries<C>(db: &C, search_name: &str) -> Result<usize, Error>
where
    C: ConnectionMut,
{
    Ok(Bakery::find()
        .filter(bakery::Column::Name.contains(search_name))
        .all(db)
        .await?
        .len())
}

// [spec:pgorm:sem:conn.pool.get+1/test]    N concurrent checkouts, status arithmetic, recovery on drop
// [spec:pgorm:req:conn.pool.no-conn-trait+1/test]    every statement runs on an explicitly acquired connection
#[pgorm_macros::test]
pub async fn pool_parallel_transactions_poolpar() -> Result<(), Error> {
    let ctx = TestContext::new("pool_parallel_transactions_poolpar").await;
    create_tables(&ctx.db).await?;

    let idle = ctx.db.status();
    assert!(
        idle.max_size >= CONNECTIONS,
        "pool max_size {} cannot hold the {CONNECTIONS} concurrent checkouts this test needs",
        idle.max_size
    );
    assert_eq!(idle.waiting, 0);

    let mut conns = Vec::with_capacity(CONNECTIONS);
    for _ in 0..CONNECTIONS {
        conns.push(ctx.db.get().await?);
    }

    let held = ctx.db.status();
    assert_eq!(held.size, CONNECTIONS);
    assert_eq!(held.available, 0);
    assert_eq!(held.waiting, 0);

    let outcomes = join_all(conns.iter_mut().enumerate().map(|(i, conn)| async move {
        let name = format!("Parallel Bakery {i}");
        conn.transaction(async |txn| {
            insert_bakery(&txn, &name, i as f64).await?;

            assert_eq!(count_bakeries(&txn, &name).await?, 1);

            Ok::<_, Error>(())
        })
        .await
    }))
    .await;

    for (i, outcome) in outcomes.into_iter().enumerate() {
        outcome.unwrap_or_else(|err| panic!("concurrent transaction {i} must commit: {err:?}"));
    }

    let still_held = ctx.db.status();
    assert_eq!(still_held.size, CONNECTIONS);
    assert_eq!(still_held.available, 0);

    drop(conns);

    let recovered = ctx.db.status();
    assert_eq!(recovered.size, CONNECTIONS);
    assert_eq!(recovered.available, CONNECTIONS);
    assert_eq!(recovered.waiting, 0);

    let db = ctx.db.get().await?;

    let mut names: Vec<String> = Bakery::find()
        .all(&db)
        .await?
        .into_iter()
        .map(|bakery| bakery.name)
        .collect();
    names.sort();

    assert_eq!(
        names,
        (0..CONNECTIONS)
            .map(|i| format!("Parallel Bakery {i}"))
            .collect::<Vec<_>>()
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:conn.pool.get+1/test]    pooled connections are distinct sessions, not a shared one
#[pgorm_macros::test]
pub async fn pool_checkouts_are_distinct_sessions_poolpar() -> Result<(), Error> {
    let ctx = TestContext::new("pool_checkouts_are_distinct_sessions_poolpar").await;
    create_tables(&ctx.db).await?;

    let mut first = ctx.db.get().await?;
    let mut second = ctx.db.get().await?;

    // The two transactions are open at once, one inside the other's closure,
    // and neither sees the other's uncommitted row.
    first
        .transaction(async |first_txn| {
            second
                .transaction(async |second_txn| {
                    insert_bakery(&first_txn, "First Bakery", 10.4).await?;
                    insert_bakery(&second_txn, "Second Bakery", 15.0).await?;

                    assert_eq!(count_bakeries(&first_txn, "Bakery").await?, 1);
                    assert_eq!(count_bakeries(&second_txn, "Bakery").await?, 1);

                    Ok::<_, Error>(())
                })
                .await
                .expect("the second transaction commits");
            Ok::<_, Error>(())
        })
        .await
        .expect("the first transaction commits");

    let observer = ctx.db.get().await?;
    assert_eq!(count_bakeries(&observer, "Bakery").await?, 2);

    drop(observer);
    drop(second);
    drop(first);
    ctx.delete().await;

    Ok(())
}
