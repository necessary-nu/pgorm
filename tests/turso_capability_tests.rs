//! Public capability boundaries for the pinned experimental native backend.
#![cfg(feature = "turso")]

use pgorm::{
    Connection, CopyConnection, DatabaseTransaction, Error, Transact, connect_turso, notify,
};
use pgorm_migration::{Migration, MigrationName, Migrator};
use pgorm_query::{CopyBinary, CopyFromStatement, CopyText, CopyToStatement, Name};
use std::{
    error::Error as StdError,
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn postgres(error: &Error) -> &pgorm_driver::Error {
    match error {
        Error::Postgres(error) => error,
        other => panic!("expected a preserved driver error, got {other:?}"),
    }
}

fn unsupported(error: &Error, operation: &str) {
    let error = postgres(error);
    assert!(error.code().is_none(), "local refusals have no SQLSTATE");
    assert!(error.to_string().contains(operation), "{error}");
    assert!(error.to_string().contains("does not support"), "{error}");
}

fn missing_function(error: &Error, function: &str) {
    let error = postgres(error);
    let database = error.as_db_error().expect("native function diagnostic");
    assert_eq!(database.code().code(), "42883");
    assert_eq!(database.message(), format!("no such function: {function}"));
    assert!(database.source().is_some(), "keep the native error cause");
}

async fn count(db: &impl Connection) -> Result<i64, Error> {
    Ok(db
        .query_one("SELECT count(*) FROM capability_items", &[])
        .await?
        .get(0))
}

async fn refuse_row_copy(db: &mut (impl CopyConnection + Send)) {
    let input = CopyFromStatement::new(Name::runtime("missing_copy_table"), CopyBinary);
    let output = CopyToStatement::new(Name::runtime("missing_copy_table"), CopyBinary);
    unsupported(
        &db.copy_in_columns(&input)
            .await
            .expect_err("refuse before describing a missing table"),
        "COPY",
    );
    unsupported(
        &db.copy_out_columns(&output)
            .await
            .expect_err("refuse before describing a missing table"),
        "COPY",
    );
    unsupported(
        &db.copy_in_rows(&input)
            .await
            .map(drop)
            .expect_err("refuse row COPY before metadata queries"),
        "COPY",
    );
    unsupported(
        &db.copy_out_rows(&output)
            .await
            .map(drop)
            .expect_err("refuse row COPY before metadata queries"),
        "COPY",
    );
}

// [spec:pgorm:req:turso.capabilities/test]
// [spec:pgorm:req:conn.copy.rows+7/test]
#[tokio::test]
async fn dedicated_copy_refuses_without_poisoning_connection() -> Result<(), Error> {
    let pool = connect_turso(":memory:").await?;
    let mut db = pool.get().await?;
    db.batch_execute("CREATE TABLE capability_items(id INTEGER PRIMARY KEY)")
        .await?;
    let input = CopyFromStatement::new(Name::runtime("capability_items"), CopyText);
    let output = CopyToStatement::new(Name::runtime("capability_items"), CopyText);
    unsupported(
        &db.copy_in(&input).await.map(drop).expect_err("COPY FROM"),
        "COPY",
    );
    unsupported(
        &db.copy_out(&output).await.map(drop).expect_err("COPY TO"),
        "COPY",
    );
    refuse_row_copy(&mut db).await;
    assert_eq!(count(&db).await?, 0);

    db.transaction(async |tx| {
        tx.execute("INSERT INTO capability_items VALUES(1)", &[])
            .await?;
        unsupported(
            &tx.copy_in(&input).await.map(drop).expect_err("COPY FROM"),
            "COPY",
        );
        unsupported(
            &tx.copy_out(&output).await.map(drop).expect_err("COPY TO"),
            "COPY",
        );
        refuse_row_copy(tx).await;
        assert_eq!(count(tx).await?, 1);
        Ok::<_, Error>(())
    })
    .await?;
    assert_eq!(count(&db).await?, 1);
    db.execute("INSERT INTO capability_items VALUES(2)", &[])
        .await?;
    assert_eq!(count(&db).await?, 2);
    Ok(())
}

// [spec:pgorm:req:turso.capabilities/test]
#[tokio::test]
async fn generic_copy_refuses_before_any_script_effects() -> Result<(), Error> {
    let pool = connect_turso(":memory:").await?;
    let db = pool.get().await?;
    db.batch_execute("CREATE TABLE capability_items(id INTEGER)")
        .await?;
    let copy = "COPY capability_items TO STDOUT";
    assert!(matches!(
        db.query_all(copy, &[]).await,
        Err(Error::CopyStatement)
    ));
    assert!(matches!(
        db.execute(copy, &[]).await,
        Err(Error::CopyStatement)
    ));
    assert!(matches!(
        db.batch_execute("INSERT INTO capability_items VALUES(1); COPY capability_items TO STDOUT")
            .await,
        Err(Error::CopyStatement)
    ));
    assert_eq!(count(&db).await?, 0);
    assert_eq!(
        db.execute("INSERT INTO capability_items VALUES(2)", &[])
            .await?,
        1
    );
    assert_eq!(count(&db).await?, 1);
    Ok(())
}

// [spec:pgorm:req:turso.capabilities/test]
#[tokio::test]
async fn notification_refusals_identify_the_failing_layer() -> Result<(), Error> {
    let pool = connect_turso(":memory:").await?;
    let db = pool.get().await?;
    let listener = match pool.listener(NonZeroUsize::MIN).await {
        Ok(_) => panic!("native notifications cannot supply a listener"),
        Err(error) => error,
    };
    unsupported(&listener, "native notification connections");
    let sending = notify(&db, "capability_topic", "payload")
        .await
        .expect_err("the pinned engine has no pg_notify");
    missing_function(&sending, "pg_notify");
    assert_eq!(db.query_one("SELECT 7", &[]).await?.get::<_, i32>(0), 7);
    Ok(())
}

// [spec:pgorm:req:turso.capabilities/test]
#[tokio::test]
async fn commands_hide_internal_rows_and_keep_effects() -> Result<(), Error> {
    let pool = connect_turso(":memory:").await?;
    let db = pool.get().await?;
    assert!(
        db.query_all("SET application_name = 'queried'", &[])
            .await?
            .is_empty()
    );
    assert_eq!(
        db.query_one("SELECT current_setting('application_name')::text", &[])
            .await?
            .get::<_, &str>(0),
        "queried"
    );
    assert_eq!(
        db.execute("SET application_name = 'executed'", &[]).await?,
        0
    );
    assert_eq!(
        db.query_one("SELECT current_setting('application_name')::text", &[])
            .await?
            .get::<_, &str>(0),
        "executed"
    );
    assert!(
        db.query_all("CREATE TABLE capability_items(id INTEGER)", &[])
            .await?
            .is_empty()
    );
    assert_eq!(
        db.execute("INSERT INTO capability_items VALUES(1),(2)", &[])
            .await?,
        2
    );
    assert_eq!(count(&db).await?, 2);
    Ok(())
}

// [spec:pgorm:req:turso.capabilities/test]
// [spec:pgorm:req:turso.errors/test]
#[tokio::test]
async fn supported_sql_keeps_native_constraint_diagnostics() -> Result<(), Error> {
    let pool = connect_turso(":memory:").await?;
    let db = pool.get().await?;
    db.batch_execute("CREATE TABLE capability_items(id INTEGER PRIMARY KEY); INSERT INTO capability_items VALUES(1)")
        .await?;
    let error = db
        .execute("INSERT INTO capability_items VALUES(1)", &[])
        .await
        .expect_err("duplicate primary key");
    let database = postgres(&error)
        .as_db_error()
        .expect("native database error");
    assert_eq!(database.code().code(), "23505");
    assert_eq!(database.table(), Some("capability_items"));
    assert_eq!(database.constraint(), Some("capability_items_pkey"));
    assert!(database.detail().is_some());
    assert!(database.source().is_some());
    assert!(
        error.sql_error().is_some(),
        "ORM constraint classification remains available"
    );
    assert_eq!(count(&db).await?, 1);
    Ok(())
}

struct ProbeMigration(Arc<AtomicUsize>);

impl MigrationName for ProbeMigration {
    fn name(&self) -> &str {
        "native_capability_probe"
    }
}

#[async_trait::async_trait]
impl Migration for ProbeMigration {
    async fn up(&self, transaction: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        transaction
            .batch_execute("CREATE TABLE migration_effect(id INTEGER)")
            .await
    }
}

struct ProbeMigrator(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl Migrator for ProbeMigrator {
    fn migrations(&self) -> Vec<Box<dyn Migration>> {
        vec![Box::new(ProbeMigration(Arc::clone(&self.0)))]
    }
}

async fn migration_tables(db: &impl Connection) -> Result<Vec<String>, Error> {
    Ok(db.query_all(
        "SELECT relname::text FROM pg_catalog.pg_class WHERE relname IN ('pgorm_migrations', 'seaql_migrations', 'migration_effect') ORDER BY relname", &[])
        .await?.iter().map(|row| row.get(0)).collect())
}

// [spec:pgorm:req:turso.capabilities/test]
#[tokio::test]
async fn migration_status_refuses_without_creating_tables() -> Result<(), Error> {
    let pool = connect_turso(":memory:").await?;
    let db = pool.get().await?;
    let calls = Arc::new(AtomicUsize::new(0));
    let migrator = ProbeMigrator(Arc::clone(&calls));
    assert!(migration_tables(&db).await?.is_empty());
    let error = migrator
        .status(&db)
        .await
        .expect_err("missing native catalog function");
    missing_function(&error, "to_regclass");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(migration_tables(&db).await?.is_empty());
    Ok(())
}

// [spec:pgorm:req:turso.capabilities/test]
#[tokio::test]
async fn migration_up_refuses_before_ledger_or_body() -> Result<(), Error> {
    let pool = connect_turso(":memory:").await?;
    let calls = Arc::new(AtomicUsize::new(0));
    let migrator = ProbeMigrator(Arc::clone(&calls));
    let db = pool.get().await?;
    assert!(migration_tables(&db).await?.is_empty());
    drop(db);
    let error = migrator
        .up(pool.clone(), None)
        .await
        .expect_err("missing native advisory lock");
    missing_function(&error, "pg_advisory_xact_lock");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let db = pool.get().await?;
    assert!(migration_tables(&db).await?.is_empty());
    db.batch_execute("CREATE TABLE capability_items(id INTEGER)")
        .await?;
    assert_eq!(
        db.execute("INSERT INTO capability_items VALUES(1)", &[])
            .await?,
        1
    );
    assert_eq!(count(&db).await?, 1);
    Ok(())
}
