#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, bakery_chain::*, setup::*};
use pgorm::{IsolationLevel, TransactionError, TransactionMode, entity::prelude::*};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

async fn insert_bakery<C>(db: &C, name: &str, profit_margin: f64) -> Result<(), Error>
where
    C: ConnectionTrait,
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
    C: ConnectionTrait,
{
    Ok(Bakery::find()
        .filter(bakery::Column::Name.contains(search_name))
        .all(db)
        .await?
        .len())
}

async fn show<C>(db: &C, setting: &str) -> Result<String, Error>
where
    C: ConnectionTrait,
{
    let row = db
        .query_one(format!("SHOW {setting}").as_str(), &[])
        .await?;
    Ok(row.get(0))
}

fn assert_read_only_violation(err: &Error) {
    match err {
        Error::Postgres(e) => assert_eq!(
            e.as_db_error().map(|e| e.code()),
            Some(&SqlState::READ_ONLY_SQL_TRANSACTION),
        ),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

#[pgorm_macros::test]
pub async fn transaction() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_test").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    db.transaction(async |txn| {
        insert_bakery(&txn, "SeaSide Bakery", 10.4).await?;
        insert_bakery(&txn, "Top Bakery", 15.0).await?;

        assert_eq!(count_bakeries(&txn, "Bakery").await?, 2);

        Ok::<_, Error>(())
    })
    .await
    .expect("the transaction commits");

    assert_eq!(count_bakeries(&db, "Bakery").await?, 2);

    drop(db);
    ctx.delete().await;

    Ok(())
}

#[pgorm_macros::test]
pub async fn transaction_with_reference() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_with_reference_test").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    let name1 = "SeaSide Bakery";
    let name2 = "Top Bakery";
    let search_name = "Bakery";

    db.transaction(async |txn| _transaction_with_reference(txn, name1, name2, search_name).await)
        .await
        .expect("the transaction commits");

    assert_eq!(count_bakeries(&db, search_name).await?, 2);

    drop(db);
    ctx.delete().await;

    Ok(())
}

async fn _transaction_with_reference(
    txn: &DatabaseTransaction<'_>,
    name1: &str,
    name2: &str,
    search_name: &str,
) -> Result<(), Error> {
    insert_bakery(txn, name1, 10.4).await?;
    insert_bakery(txn, name2, 15.0).await?;

    assert_eq!(count_bakeries(txn, search_name).await?, 2);

    Ok(())
}

/// Run a transaction on `db` that inserts `names` and then never finishes,
/// dropping its future once the inserts are in, as a timeout or a cancelled
/// task would.
async fn insert_then_drop(db: &mut DatabaseConnection, names: &[&str]) {
    let (inserted, ready) = tokio::sync::oneshot::channel();
    let transaction = db.transaction(async |txn| {
        for name in names {
            insert_bakery(&txn, name, 10.4).await?;
        }
        assert_eq!(count_bakeries(&txn, "Bakery").await?, names.len());
        inserted.send(()).expect("the test is waiting");

        std::future::pending::<Result<(), Error>>().await
    });

    tokio::select! {
        outcome = transaction => panic!("the transaction cannot finish: {outcome:?}"),
        ready = ready => ready.expect("the closure signals its inserts"),
    }
}

// [spec:pgorm:sem:conn.tx.guard+3/test]    a dropped closure future rolls back, and the queued ROLLBACK lands ahead of the connection's next statement
#[pgorm_macros::test]
pub async fn transaction_dropped_future_rolls_back() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_dropped_future_rolls_back").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    assert_eq!(bakery::Entity::find().all(&db).await?.len(), 0);

    insert_then_drop(&mut db, &["SeaSide Bakery", "Top Bakery"]).await;

    assert_eq!(bakery::Entity::find().all(&db).await?.len(), 0);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:conn.tx.guard+3/test]    a closure stopped by a timeout rolls back
#[pgorm_macros::test]
pub async fn transaction_timed_out_rolls_back() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_timed_out_rolls_back").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    // Whether the insert is in when the timeout fires or still on the wire,
    // dropping the future leaves the rollback to the handle, so the outcome
    // does not depend on how fast the server answers.
    let elapsed = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        db.transaction(async |txn| {
            insert_bakery(&txn, "Slow Bakery", 10.4).await?;
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            Ok::<_, Error>(())
        }),
    )
    .await;

    assert!(elapsed.is_err(), "the timeout ends the transaction first");
    assert_eq!(count_bakeries(&db, "Bakery").await?, 0);

    drop(db);
    ctx.delete().await;

    Ok(())
}

#[pgorm_macros::test]
pub async fn transaction_commits_on_ok() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_commits_on_ok").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    assert_eq!(bakery::Entity::find().all(&db).await?.len(), 0);

    db.transaction(async |txn| {
        insert_bakery(&txn, "SeaSide Bakery", 10.4).await?;

        assert_eq!(bakery::Entity::find().all(&txn).await?.len(), 1);

        insert_bakery(&txn, "Top Bakery", 15.0).await?;

        assert_eq!(bakery::Entity::find().all(&txn).await?.len(), 2);

        Ok::<_, Error>(())
    })
    .await
    .expect("the transaction commits");

    assert_eq!(bakery::Entity::find().all(&db).await?.len(), 2);

    drop(db);
    ctx.delete().await;

    Ok(())
}

#[pgorm_macros::test]
pub async fn transaction_error_rollback() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_error_rollback_test").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    assert_eq!(bakery::Entity::find().all(&db).await?.len(), 0);

    let outcome = db
        .transaction(async |txn| {
            insert_bakery(&txn, "SeaSide Bakery", 10.4).await?;
            insert_bakery(&txn, "Top Bakery", 15.0).await?;

            assert_eq!(bakery::Entity::find().all(&txn).await?.len(), 2);

            bakery::ActiveModel {
                id: set(1),
                name: set("Duplicated primary key"),
                profit_margin: set(20.0),
            }
            .insert(&txn)
            .await
        })
        .await;

    assert!(
        matches!(
            outcome,
            Err(TransactionError::Transaction(Error::Postgres(_)))
        ),
        "the duplicate key fails the closure: {outcome:?}"
    );
    assert_eq!(bakery::Entity::find().all(&db).await?.len(), 0);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:conn.tx.guard+3/test]    an Err rolls back, and the ROLLBACK is awaited before the call returns
#[pgorm_macros::test]
pub async fn transaction_err_rolls_back() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_err_rolls_back_txrollback").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    assert_eq!(count_bakeries(&db, "Bakery").await?, 0);

    let outcome = db
        .transaction(async |txn| {
            insert_bakery(&txn, "SeaSide Bakery", 10.4).await?;
            insert_bakery(&txn, "Top Bakery", 15.0).await?;

            assert_eq!(count_bakeries(&txn, "Bakery").await?, 2);

            roll_back::<()>()
        })
        .await;

    assert_rolled_back(outcome);
    assert_eq!(count_bakeries(&db, "Bakery").await?, 0);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:conn.tx.guard+3/test]    the ROLLBACK a dropped future queues still lands after the connection returns to the pool
#[pgorm_macros::test]
pub async fn transaction_drop_rollback_survives_handback() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_drop_rollback_survives_handback").await;
    create_tables(&ctx.db).await?;

    let base_url =
        std::env::var("DATABASE_URL").expect("Enviroment variable 'DATABASE_URL' not set");
    // One connection only, so the next checkout is guaranteed to be the very
    // connection the dropped transaction queued its ROLLBACK on.
    let pool =
        pgorm::connect_with_builder(common::setup::config(&base_url, ctx.db_name()), |builder| {
            builder.max_size(1)
        })?;

    let mut db = pool.get().await?;
    let pid: i32 = db.query_one("SELECT pg_backend_pid()", &[]).await?.get(0);

    // Dropped unfinished: the handle's Drop enqueues ROLLBACK and returns at
    // once, without awaiting the server's response.
    insert_then_drop(&mut db, &["Dropped Bakery"]).await;

    drop(db);

    let db = pool.get().await?;
    let same_pid: i32 = db.query_one("SELECT pg_backend_pid()", &[]).await?.get(0);
    assert_eq!(
        same_pid, pid,
        "a max_size 1 pool hands the same connection to the next caller"
    );
    assert_eq!(
        count_bakeries(&db, "Bakery").await?,
        0,
        "the queued ROLLBACK is drained before the next caller's work"
    );

    drop(db);
    drop(pool);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:conn.tx.guard+3/test]    a failing COMMIT reaches the caller as TransactionError::Connection(Error::Postgres)
#[pgorm_macros::test]
pub async fn transaction_commit_failure_maps_error() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_commit_failure_maps_error").await;
    let mut db = ctx.db.get().await?;

    // A deferred constraint is only checked at COMMIT, so the failure can reach
    // nothing but the commit itself.
    db.batch_execute(
        "CREATE TABLE deferred_probe (id int, CONSTRAINT deferred_probe_key UNIQUE (id) \
         DEFERRABLE INITIALLY DEFERRED)",
    )
    .await?;

    let err = db
        .transaction(async |txn| {
            txn.execute("INSERT INTO deferred_probe VALUES (1)", &[])
                .await?;
            txn.execute("INSERT INTO deferred_probe VALUES (1)", &[])
                .await?;
            Ok::<_, Error>(())
        })
        .await
        .expect_err("the deferred unique constraint must fail the COMMIT");

    match &err {
        TransactionError::Connection(Error::Postgres(e)) => assert_eq!(
            e.as_db_error().map(|e| e.code()),
            Some(&SqlState::UNIQUE_VIOLATION),
        ),
        other => panic!("expected TransactionError::Connection(Error::Postgres), got {other:?}"),
    }

    assert!(
        db.query_all("SELECT id FROM deferred_probe", &[])
            .await?
            .is_empty(),
        "a failed COMMIT leaves nothing behind"
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:conn.tx+3/test]    ReadOnly emits both clauses, and the server enforces the access mode
#[pgorm_macros::test]
pub async fn transaction_read_only_txconfig() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_read_only_txconfig").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    let read_only = TransactionMode::ReadOnly {
        isolation: Some(IsolationLevel::RepeatableRead),
    };
    let outcome = db
        .transaction_with(read_only, async |txn| {
            assert_eq!(
                show(&txn, "transaction_isolation").await?,
                "repeatable read"
            );
            assert_eq!(show(&txn, "transaction_read_only").await?, "on");

            let err = insert_bakery(&txn, "Read Only Bakery", 10.4)
                .await
                .expect_err("INSERT must be rejected in a read-only transaction");

            assert_read_only_violation(&err);

            roll_back::<()>()
        })
        .await;

    assert_rolled_back(outcome);
    assert_eq!(count_bakeries(&db, "Bakery").await?, 0);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:conn.tx+3/test]    ReadWrite carries the isolation level, and still commits
#[pgorm_macros::test]
pub async fn transaction_serializable_txconfig() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_serializable_txconfig").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    db.transaction_with(serializable(), async |txn| {
        assert_eq!(show(&txn, "transaction_isolation").await?, "serializable");
        assert_eq!(show(&txn, "transaction_read_only").await?, "off");

        insert_bakery(&txn, "SeaSide Bakery", 10.4).await?;

        assert_eq!(count_bakeries(&txn, "Bakery").await?, 1);

        Ok::<_, Error>(())
    })
    .await
    .expect("the serializable transaction commits");

    assert_eq!(count_bakeries(&db, "Bakery").await?, 1);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:conn.tx+3/test]    Default appends no clause, so the session default stands
#[pgorm_macros::test]
pub async fn transaction_default_inherits_session_txconfig() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_default_inherits_session_txconfig").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    db.batch_execute("SET default_transaction_read_only = on")
        .await?;

    let outcome = db
        .transaction_with(TransactionMode::Default, async |txn| {
            assert_eq!(show(&txn, "transaction_read_only").await?, "on");

            let err = insert_bakery(&txn, "Read Only Bakery", 10.4)
                .await
                .expect_err("Default must inherit the session's read-only default");

            assert_read_only_violation(&err);

            roll_back::<()>()
        })
        .await;

    assert_rolled_back(outcome);

    db.batch_execute("SET default_transaction_read_only = off")
        .await?;

    assert_eq!(count_bakeries(&db, "Bakery").await?, 0);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:conn.tx+3/test]    ReadWrite overrides a read-only session default
#[pgorm_macros::test]
pub async fn transaction_read_write_overrides_default_txconfig() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_read_write_overrides_default_txconfig").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    db.batch_execute("SET default_transaction_read_only = on")
        .await?;

    db.transaction_with(
        TransactionMode::ReadWrite { isolation: None },
        async |txn| {
            assert_eq!(show(&txn, "transaction_read_only").await?, "off");

            insert_bakery(&txn, "SeaSide Bakery", 10.4).await
        },
    )
    .await
    .expect("the read-write transaction commits");

    db.batch_execute("SET default_transaction_read_only = off")
        .await?;

    assert_eq!(count_bakeries(&db, "Bakery").await?, 1);

    drop(db);
    ctx.delete().await;

    Ok(())
}

#[pgorm_macros::test]
pub async fn transaction_with_active_model_behaviour() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_with_active_model_behaviour_test").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    let outcome = db
        .transaction(async |txn| {
            assert_eq!(
                cake::ActiveModel {
                    name: set("Cake with invalid price"),
                    price: set(rust_dec(0)),
                    gluten_free: set(false),
                    ..Default::default()
                }
                .insert(&txn)
                .await,
                Err(Error::Custom(
                    "[before_save] Invalid Price, insert: true".to_owned()
                ))
            );

            assert_eq!(cake::Entity::find().all(&txn).await?.len(), 0);

            assert_eq!(
                cake::ActiveModel {
                    name: set("Cake with invalid price"),
                    price: set(rust_dec(-10)),
                    gluten_free: set(false),
                    ..Default::default()
                }
                .insert(&txn)
                .await,
                Err(Error::Custom(
                    "[after_save] Invalid Price, insert: true".to_owned()
                ))
            );

            assert_eq!(cake::Entity::find().all(&txn).await?.len(), 1);

            let readonly_cake_1 = cake::ActiveModel {
                name: set("Readonly cake (err_on_before_delete)"),
                price: set(rust_dec(10)),
                gluten_free: set(true),
                ..Default::default()
            }
            .insert(&txn)
            .await?;

            assert_eq!(cake::Entity::find().all(&txn).await?.len(), 2);

            assert_eq!(
                readonly_cake_1.delete(&txn).await.err(),
                Some(Error::Custom(
                    "[before_delete] Cannot be deleted".to_owned()
                ))
            );

            assert_eq!(cake::Entity::find().all(&txn).await?.len(), 2);

            let readonly_cake_2 = cake::ActiveModel {
                name: set("Readonly cake (err_on_after_delete)"),
                price: set(rust_dec(10)),
                gluten_free: set(true),
                ..Default::default()
            }
            .insert(&txn)
            .await?;

            assert_eq!(cake::Entity::find().all(&txn).await?.len(), 3);

            assert_eq!(
                readonly_cake_2.delete(&txn).await.err(),
                Some(Error::Custom("[after_delete] Cannot be deleted".to_owned()))
            );

            assert_eq!(cake::Entity::find().all(&txn).await?.len(), 2);

            roll_back::<()>()
        })
        .await;

    assert_rolled_back(outcome);
    assert_eq!(cake::Entity::find().all(&db).await?.len(), 0);

    drop(db);
    ctx.delete().await;

    Ok(())
}

#[pgorm_macros::test]
pub async fn transaction_nested() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_nested_test").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    db.transaction(async |txn| {
        insert_bakery(&txn, "SeaSide Bakery", 10.4).await?;
        insert_bakery(&txn, "Top Bakery", 15.0).await?;

        assert_eq!(count_bakeries(&txn, "Bakery").await?, 2);

        // Nested transaction (savepoint) that gets committed
        txn.transaction(async |txn2| {
            insert_bakery(&txn2, "Nested Bakery", 88.88).await?;

            assert_eq!(count_bakeries(&txn2, "Bakery").await?, 3);

            // Nested-nested transaction rolled back
            let outcome = txn2
                .transaction(async |txn3| {
                    insert_bakery(&txn3, "Rock n Roll Bakery", 28.8).await?;

                    assert_eq!(count_bakeries(&txn3, "Bakery").await?, 4);

                    roll_back::<()>()
                })
                .await;
            assert_rolled_back(outcome);

            assert_eq!(count_bakeries(&txn2, "Bakery").await?, 3);

            // Nested-nested transaction committed
            txn2.transaction(async |txn3| {
                insert_bakery(&txn3, "Rock n Roll Bakery", 28.8).await?;

                assert_eq!(count_bakeries(&txn3, "Bakery").await?, 4);

                Ok::<_, Error>(())
            })
            .await
            .expect("the savepoint commits");

            assert_eq!(count_bakeries(&txn2, "Bakery").await?, 4);

            Ok::<_, Error>(())
        })
        .await
        .expect("the savepoint commits");

        assert_eq!(count_bakeries(&txn, "Bakery").await?, 4);

        // Nested transaction (savepoint) rolled back
        let outcome = txn
            .transaction(async |txn2| {
                insert_bakery(&txn2, "Rock n Roll Bakery", 28.8).await?;

                assert_eq!(count_bakeries(&txn2, "Bakery").await?, 5);

                // Nested-nested transaction committed
                txn2.transaction(async |txn3| {
                    insert_bakery(&txn3, "Rock n Roll Bakery", 28.8).await?;

                    assert_eq!(count_bakeries(&txn3, "Bakery").await?, 6);

                    Ok::<_, Error>(())
                })
                .await
                .expect("the savepoint commits");

                assert_eq!(count_bakeries(&txn2, "Bakery").await?, 6);

                // Nested-nested transaction rolled back
                let outcome = txn2
                    .transaction(async |txn3| {
                        insert_bakery(&txn3, "Rock n Roll Bakery", 28.8).await?;

                        assert_eq!(count_bakeries(&txn3, "Bakery").await?, 7);

                        roll_back::<()>()
                    })
                    .await;
                assert_rolled_back(outcome);

                assert_eq!(count_bakeries(&txn2, "Bakery").await?, 6);

                roll_back::<()>()
            })
            .await;
        assert_rolled_back(outcome);

        assert_eq!(count_bakeries(&txn, "Bakery").await?, 4);

        Ok::<_, Error>(())
    })
    .await
    .expect("the transaction commits");

    assert_eq!(count_bakeries(&db, "Bakery").await?, 4);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:conn.tx.closure+5/test]    a savepoint nested in a connection transaction, both plain closures, runs in a spawned task and commits
#[pgorm_macros::test]
pub async fn transaction_nested_in_spawned_task() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_nested_in_spawned_task").await;
    create_tables(&ctx.db).await?;

    let pool = ctx.db.clone();
    let counted = tokio::spawn(async move {
        let mut db = pool.get().await?;
        db.transaction(async |txn| {
            insert_bakery(&txn, "Outer Bakery", 1.0).await?;
            txn.transaction(async |savepoint| {
                insert_bakery(&savepoint, "Savepoint Bakery", 2.0).await?;
                count_bakeries(&savepoint, "Bakery").await
            })
            .await
            .map_err(|e| Error::Custom(e.to_string()))
        })
        .await
        .map_err(|e| Error::Custom(e.to_string()))
    })
    .await
    .expect("the spawned task completes")?;

    assert_eq!(counted, 2);
    assert_eq!(count_bakeries(&ctx.db.get().await?, "Bakery").await?, 2);

    ctx.delete().await;

    Ok(())
}

fn serializable() -> TransactionMode {
    TransactionMode::ReadWrite {
        isolation: Some(IsolationLevel::Serializable),
    }
}

// [spec:pgorm:sem:conn.tx.closure+5/test]    Ok commits, and the value is returned
#[pgorm_macros::test]
pub async fn transaction_closure_commit_txclosure() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_closure_commit_txclosure").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    let inserted = db
        .transaction(async |txn| {
            insert_bakery(&*txn, "SeaSide Bakery", 10.4).await?;
            insert_bakery(&*txn, "Top Bakery", 15.0).await?;
            count_bakeries(&*txn, "Bakery").await
        })
        .await
        .expect("closure transaction must commit");

    assert_eq!(inserted, 2);
    assert_eq!(count_bakeries(&db, "Bakery").await?, 2);

    let counted = db
        .transaction_with(serializable(), async |txn| {
            count_bakeries(&*txn, "Bakery").await
        })
        .await
        .expect("configured closure transaction must commit");

    assert_eq!(counted, 2);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:conn.tx.closure+5/test]    each closure form's future is Send, so it runs in a spawned task
#[pgorm_macros::test]
pub async fn transaction_closures_run_in_spawned_tasks() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_closures_run_in_spawned_tasks").await;
    create_tables(&ctx.db).await?;

    let pool = ctx.db.clone();
    tokio::spawn(async move {
        let mut db = pool.get().await?;
        db.transaction(async |txn| insert_bakery(&*txn, "Plain Bakery", 1.0).await)
            .await
            .map_err(|e| Error::Custom(e.to_string()))
    })
    .await
    .expect("the spawned task completes")?;

    let pool = ctx.db.clone();
    tokio::spawn(async move {
        let mut db = pool.get().await?;
        db.transaction_with(serializable(), async |txn| {
            insert_bakery(&*txn, "Configured Bakery", 2.0).await
        })
        .await
        .map_err(|e| Error::Custom(e.to_string()))
    })
    .await
    .expect("the spawned task completes")?;

    let pool = ctx.db.clone();
    tokio::spawn(async move {
        let mut db = pool.get().await?;
        db.transaction_with_retry(serializable(), 3, async |txn| {
            insert_bakery(&*txn, "Retried Bakery", 3.0).await
        })
        .await
        .map_err(|e| Error::Custom(e.to_string()))
    })
    .await
    .expect("the spawned task completes")?;

    assert_eq!(count_bakeries(&ctx.db.get().await?, "Bakery").await?, 3);

    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:conn.tx.closure+5/test]    Err rolls back and is wrapped, not swallowed
#[pgorm_macros::test]
pub async fn transaction_closure_error_txclosure() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_closure_error_txclosure").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    let err = db
        .transaction(async |txn| {
            insert_bakery(&*txn, "SeaSide Bakery", 10.4).await?;
            insert_bakery(&*txn, "Top Bakery", 15.0).await?;

            assert_eq!(count_bakeries(&*txn, "Bakery").await?, 2);

            Err::<(), Error>(Error::Custom("closure said no".to_owned()))
        })
        .await
        .expect_err("closure transaction must not commit");

    match err {
        TransactionError::Transaction(Error::Custom(message)) => {
            assert_eq!(message, "closure said no")
        }
        other => panic!("expected TransactionError::Transaction, got {other:?}"),
    }

    assert_eq!(count_bakeries(&db, "Bakery").await?, 0);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:conn.tx.closure+5/test]    a ROLLBACK that fails after the closure's error is reported beside it
#[pgorm_macros::test]
pub async fn transaction_failed_rollback_is_reported() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_failed_rollback_is_reported").await;
    let mut db = ctx.db.get().await?;
    let killer = ctx.db.get().await?;
    let pid: i32 = db.query_one("SELECT pg_backend_pid()", &[]).await?.get(0);

    // The backend ends while the transaction is idle, so the ROLLBACK that
    // follows the closure's error has nobody to answer it.
    let outcome = db
        .transaction(async |txn| {
            txn.execute("SELECT 1", &[]).await?;
            killer
                .execute("SELECT pg_terminate_backend($1, 5000)", &[&pid])
                .await?;
            Err::<(), Error>(Error::Custom("closure said no".to_owned()))
        })
        .await;

    match outcome {
        Err(TransactionError::RollbackFailed {
            cause: Error::Custom(message),
            rollback: Error::Postgres(_),
        }) => assert_eq!(message, "closure said no"),
        other => panic!("expected TransactionError::RollbackFailed, got {other:?}"),
    }

    db.discard();
    drop(killer);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:conn.tx.compose/test]    `?` passes a savepoint's failure and its value to the closure around it
#[pgorm_macros::test]
pub async fn savepoint_composes_with_question_mark() -> Result<(), Error> {
    let ctx = TestContext::new("savepoint_composes_with_question_mark").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    let refused = db
        .transaction(async |txn| {
            insert_bakery(&txn, "Outer Bakery", 1.0).await?;
            txn.transaction(async |savepoint| {
                insert_bakery(&savepoint, "Inner Bakery", 2.0).await?;
                Err::<(), Error>(Error::Custom("savepoint said no".to_owned()))
            })
            .await?;
            insert_bakery(&txn, "Unreached Bakery", 3.0).await
        })
        .await;
    match refused {
        Err(TransactionError::Transaction(Error::Custom(message))) => {
            assert_eq!(message, "savepoint said no")
        }
        other => panic!("expected the savepoint's error as the closure's, got {other:?}"),
    }
    assert_eq!(
        count_bakeries(&db, "Bakery").await?,
        0,
        "the enclosing transaction rolled back with the savepoint's error"
    );

    // A function returning `Error` takes a whole transaction's outcome with `?`.
    let counted = db
        .transaction(async |txn| {
            let counted = txn
                .transaction(async |savepoint| {
                    insert_bakery(&savepoint, "Kept Bakery", 4.0).await?;
                    count_bakeries(&savepoint, "Bakery").await
                })
                .await?;
            Ok::<_, Error>(counted)
        })
        .await?;
    assert_eq!(counted, 1);
    assert_eq!(count_bakeries(&db, "Kept Bakery").await?, 1);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:conn.tx.compose/test]    a savepoint's failed ROLLBACK reaches the caller beside its cause
#[pgorm_macros::test]
pub async fn savepoint_failed_rollback_is_not_lost() -> Result<(), Error> {
    let ctx = TestContext::new("savepoint_failed_rollback_is_not_lost").await;
    let mut db = ctx.db.get().await?;
    let killer = ctx.db.get().await?;
    let pid: i32 = db.query_one("SELECT pg_backend_pid()", &[]).await?.get(0);

    // The backend ends inside the savepoint, so neither its ROLLBACK TO
    // SAVEPOINT nor the enclosing transaction's ROLLBACK is answered.
    let outcome = db
        .transaction(async |txn| {
            txn.transaction(async |savepoint| {
                savepoint.execute("SELECT 1", &[]).await?;
                killer
                    .execute(
                        "SELECT pg_terminate_backend(pid, 5000) FROM pg_stat_activity \
                         WHERE pid = $1 AND datname = current_database()",
                        &[&pid],
                    )
                    .await?;
                Err::<(), Error>(Error::Custom("savepoint said no".to_owned()))
            })
            .await?;
            Ok::<_, Error>(())
        })
        .await;

    match outcome {
        Err(TransactionError::RollbackFailed {
            cause: Error::RollbackFailed { cause, rollback },
            rollback: Error::Postgres(_),
        }) => {
            assert!(
                matches!(*cause, Error::Custom(ref message) if message == "savepoint said no"),
                "{cause:?}"
            );
            assert!(matches!(*rollback, Error::Postgres(_)), "{rollback:?}");
        }
        other => panic!("expected both failed rollbacks beside the cause, got {other:?}"),
    }

    db.discard();
    drop(killer);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:conn.tx.retry+3/test]    SQLSTATE 40001 raised by a statement is retried
#[pgorm_macros::test]
pub async fn transaction_retry_concurrent_update_txclosure() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_retry_concurrent_update_txclosure").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    insert_bakery(&db, "SeaSide Bakery", 10.4).await?;

    let pool = ctx.db.clone();
    let mut attempts = 0u32;

    let read = db
        .transaction_with_retry(serializable(), 3, async |txn| {
            attempts += 1;

            let row = txn
                .query_one(
                    "SELECT profit_margin FROM bakery WHERE name = $1",
                    &[&"SeaSide Bakery"],
                )
                .await?;
            let margin: f64 = row.get(0);

            if attempts == 1 {
                let mut other = pool.get().await?;
                other
                    .transaction(async |other_txn| {
                        other_txn
                            .execute(
                                "UPDATE bakery SET profit_margin = 99.0 WHERE name = $1",
                                &[&"SeaSide Bakery"],
                            )
                            .await
                    })
                    .await
                    .expect("the concurrent update commits");
            }

            txn.execute(
                "UPDATE bakery SET profit_margin = $1 WHERE name = $2",
                &[&(margin + 1.0), &"SeaSide Bakery"],
            )
            .await?;

            Ok::<_, Error>(margin)
        })
        .await
        .expect("the retried transaction must eventually commit");

    assert_eq!(attempts, 2);
    assert_eq!(read, 99.0);

    let bakery = Bakery::find().one(&db).await?;
    assert_eq!(bakery.profit_margin, 100.0);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:conn.tx.retry+3/test]    write skew between two serializable transactions
#[pgorm_macros::test]
pub async fn transaction_retry_write_skew_txclosure() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_retry_write_skew_txclosure").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    let pool = ctx.db.clone();
    let mut attempts = 0u32;

    let seen = db
        .transaction_with_retry(serializable(), 3, async |txn| {
            attempts += 1;

            let seen = count_bakeries(&*txn, "Bakery").await?;
            insert_bakery(&*txn, "Top Bakery", 15.0).await?;

            if attempts == 1 {
                let mut other = pool.get().await?;
                other
                    .transaction_with(serializable(), async |other_txn| {
                        count_bakeries(&other_txn, "Bakery").await?;
                        insert_bakery(&other_txn, "Rival Bakery", 20.0).await
                    })
                    .await
                    .expect("the rival transaction commits");
            }

            Ok::<_, Error>(seen)
        })
        .await
        .expect("the retried transaction must eventually commit");

    assert_eq!(attempts, 2);
    assert_eq!(seen, 1);
    assert_eq!(count_bakeries(&db, "Bakery").await?, 2);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:conn.tx.retry+3/test]    a non-retryable error is returned on the first attempt
#[pgorm_macros::test]
pub async fn transaction_retry_non_retryable_txclosure() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_retry_non_retryable_txclosure").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    let mut attempts = 0u32;

    let err = db
        .transaction_with_retry(serializable(), 5, async |txn| {
            attempts += 1;
            insert_bakery(&*txn, "SeaSide Bakery", 10.4).await?;
            Err::<(), Error>(Error::Custom("closure said no".to_owned()))
        })
        .await
        .expect_err("a non-retryable error must not be retried");

    assert_eq!(attempts, 1);
    match err {
        TransactionError::Transaction(Error::Custom(message)) => {
            assert_eq!(message, "closure said no")
        }
        other => panic!("expected TransactionError::Transaction, got {other:?}"),
    }

    assert_eq!(count_bakeries(&db, "Bakery").await?, 0);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:conn.tx+3/test]    a savepoint under a configured outer transaction
#[pgorm_macros::test]
pub async fn transaction_nested_under_config_txtests() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_nested_under_config_txtests").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    db.transaction_with(serializable(), async |txn| {
        insert_bakery(&txn, "SeaSide Bakery", 10.4).await?;

        let outcome = txn
            .transaction(async |txn2| {
                insert_bakery(&txn2, "Rolled Back Bakery", 88.88).await?;

                assert_eq!(count_bakeries(&txn2, "Bakery").await?, 2);

                roll_back::<()>()
            })
            .await;
        assert_rolled_back(outcome);

        assert_eq!(count_bakeries(&txn, "Bakery").await?, 1);

        txn.transaction(async |txn2| {
            insert_bakery(&txn2, "Top Bakery", 15.0).await?;

            assert_eq!(count_bakeries(&txn2, "Bakery").await?, 2);

            Ok::<_, Error>(())
        })
        .await
        .expect("the savepoint commits");

        assert_eq!(count_bakeries(&txn, "Bakery").await?, 2);

        Ok::<_, Error>(())
    })
    .await
    .expect("the configured transaction commits");

    assert_eq!(count_bakeries(&db, "Bakery").await?, 2);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:conn.tx+3/test]    DeferrableSnapshot carries serializable + read only, the one combination DEFERRABLE needs
#[pgorm_macros::test]
pub async fn transaction_deferrable_txtests() -> Result<(), Error> {
    let ctx = TestContext::new("transaction_deferrable_txtests").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    insert_bakery(&db, "SeaSide Bakery", 10.4).await?;

    db.transaction_with(TransactionMode::DeferrableSnapshot, async |txn| {
        assert_eq!(show(&txn, "transaction_isolation").await?, "serializable");
        assert_eq!(show(&txn, "transaction_read_only").await?, "on");
        assert_eq!(show(&txn, "transaction_deferrable").await?, "on");

        assert_eq!(count_bakeries(&txn, "Bakery").await?, 1);

        Ok::<_, Error>(())
    })
    .await
    .expect("the deferrable snapshot commits");

    assert_eq!(count_bakeries(&db, "Bakery").await?, 1);

    drop(db);
    ctx.delete().await;

    Ok(())
}
