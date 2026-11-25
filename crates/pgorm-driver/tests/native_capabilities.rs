//! Reproducible capability evidence for the pinned embedded frontend.
#![cfg(feature = "turso")]

use pgorm_driver::{TursoDatabase, error::SqlState, types::Type};
use std::{error::Error as StdError, num::NonZeroUsize, time::Duration};
use turso_core::Value;
use turso_pg::{PgConnection, StatementResultKind};

type TestResult = Result<(), Box<dyn StdError + Send + Sync>>;

const CLASS: &str = "SELECT relname, relkind FROM pg_catalog.pg_class WHERE relname = 'cap_probe'";
const ATTRIBUTE: &str = "SELECT attname, attnum FROM pg_catalog.pg_attribute WHERE attrelid = (SELECT oid FROM pg_catalog.pg_class WHERE relname = 'cap_probe') AND attnum > 0 ORDER BY attnum";
const TYPES: &str = "SELECT typname, typtype FROM pg_catalog.pg_type WHERE typname IN ('int4', 'text') ORDER BY typname";
const TABLES: &str =
    "SELECT table_schema, table_name FROM information_schema.tables WHERE table_name = 'cap_probe'";
const COLUMNS: &str = "SELECT column_name, data_type FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'cap_probe' ORDER BY ordinal_position";
const LEDGER: &str = "SELECT to_regclass(concat_ws('.', quote_ident($1), quote_ident($2))) IS NOT NULL, to_regclass(quote_ident($3)) IS NOT NULL";
const ADOPT: &str = "SELECT to_regclass($1) IS NULL AND to_regclass($2) IS NOT NULL";
const CHECKSUM: &str = "SELECT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema = coalesce($1, current_schema()) AND table_name = $2 AND column_name = $3)";

#[derive(Debug)]
struct NativeResult {
    kind: StatementResultKind,
    columns: Vec<(String, Option<String>)>,
    rows: Vec<Vec<Value>>,
}

fn native_connection() -> Result<PgConnection, turso_pg::Error> {
    let (_, database) = turso_pg::open_database(
        ":memory:",
        None,
        turso_pg::OpenFlags::default(),
        turso_pg::tursopg_database_opts(),
    )?;
    PgConnection::connect(&database)
}

fn native_query(
    connection: &PgConnection,
    sql: &str,
    params: &[Value],
) -> Result<NativeResult, turso_pg::Error> {
    let mut statement = connection.prepare(sql)?;
    let kind = statement.result_kind();
    let columns = (0..statement.num_columns())
        .map(|index| {
            Ok((
                statement.get_column_name(index).to_string(),
                statement
                    .get_column_type_info(index)?
                    .map(|info| info.declared_name),
            ))
        })
        .collect::<Result<_, turso_pg::Error>>()?;
    for (index, value) in params.iter().enumerate() {
        let position = NonZeroUsize::MIN
            .checked_add(index)
            .ok_or(turso_core::LimboError::IntegerOverflow)?;
        statement.bind_at(position, value.clone())?;
    }
    let rows = statement.run_collect_rows()?;
    statement.reset()?;
    Ok(NativeResult {
        kind,
        columns,
        rows,
    })
}

// [spec:pgorm:req:turso.capabilities/test]
#[test]
fn native_catalog_queries_preserve_fixture_values() -> TestResult {
    let connection = native_connection()?;
    connection.execute("CREATE TABLE cap_probe(id INTEGER, label TEXT)")?;
    for (sql, expected_columns, expected_rows) in [
        (
            CLASS,
            vec![("relname", "TEXT"), ("relkind", "TEXT")],
            vec![vec![Value::build_text("cap_probe"), Value::build_text("r")]],
        ),
        (
            ATTRIBUTE,
            vec![("attname", "TEXT"), ("attnum", "INTEGER")],
            vec![
                vec![Value::build_text("id"), Value::from_i64(1)],
                vec![Value::build_text("label"), Value::from_i64(2)],
            ],
        ),
        (
            TYPES,
            vec![("typname", "TEXT"), ("typtype", "TEXT")],
            vec![
                vec![Value::build_text("int4"), Value::build_text("b")],
                vec![Value::build_text("text"), Value::build_text("b")],
            ],
        ),
    ] {
        let result = native_query(&connection, sql, &[])?;
        assert_eq!(result.kind, StatementResultKind::Rows);
        assert_eq!(result.rows, expected_rows, "{sql}");
        assert_eq!(
            result.columns,
            expected_columns
                .into_iter()
                .map(|(name, ty)| (name.to_owned(), Some(ty.to_owned())))
                .collect::<Vec<_>>()
        );
    }
    for (sql, message) in [
        (TABLES, "no such table: main.tables"),
        (COLUMNS, "no such table: main.columns"),
        (CHECKSUM, "no such table: main.columns"),
    ] {
        let error =
            native_query(&connection, sql, &[]).expect_err("catalog gap must remain explicit");
        assert_eq!(error.sqlstate, "42P01");
        assert_eq!(error.to_string(), message);
    }
    connection.close()?;
    Ok(())
}

// [spec:pgorm:req:turso.capabilities/test]
#[test]
fn native_migration_functions_fail_before_binding() -> TestResult {
    let connection = native_connection()?;
    connection.execute("CREATE TABLE cap_probe(id INTEGER)")?;
    for (sql, function) in [
        ("SELECT pg_notify('cap_topic', 'payload')", "pg_notify"),
        ("SELECT pg_notify($1, $2)", "pg_notify"),
        ("SELECT pg_advisory_xact_lock(7)", "pg_advisory_xact_lock"),
        ("SELECT pg_advisory_xact_lock($1)", "pg_advisory_xact_lock"),
        ("SELECT to_regclass('public.cap_probe')", "to_regclass"),
        ("SELECT to_regclass($1)", "to_regclass"),
        (ADOPT, "to_regclass"),
        (LEDGER, "to_regclass"),
    ] {
        // These native preparation errors precede parameter binding and driver inference.
        let error = native_query(&connection, sql, &[]).expect_err("native function missing");
        assert_eq!(error.sqlstate, "42883", "{sql}");
        assert_eq!(error.to_string(), format!("no such function: {function}"));
    }
    connection.close()?;
    Ok(())
}

// [spec:pgorm:req:turso.capabilities/test]
#[test]
fn native_command_kinds_expose_internal_metadata() -> TestResult {
    let connection = native_connection()?;
    for sql in [
        "LISTEN cap_topic",
        "NOTIFY cap_topic, 'payload'",
        "UNLISTEN *",
    ] {
        let error = native_query(&connection, sql, &[]).expect_err("notifications unsupported");
        assert_eq!(error.sqlstate, "0A000");
    }
    for sql in ["CREATE SCHEMA cap_schema", "RESET ALL"] {
        let result = native_query(&connection, sql, &[])?;
        assert_eq!(result.kind, StatementResultKind::Command);
        assert_eq!(
            result.columns,
            [("?column?".to_owned(), Some("INTEGER".to_owned()))]
        );
        assert!(result.rows.is_empty());
    }
    let result = native_query(&connection, "CREATE TABLE cap_probe(id INTEGER)", &[])?;
    assert_eq!(result.kind, StatementResultKind::Command);
    assert!(result.columns.is_empty());
    assert!(result.rows.is_empty());
    connection.execute("INSERT INTO cap_probe VALUES (7)")?;
    let result = native_query(&connection, "COPY cap_probe TO STDOUT", &[])?;
    assert_eq!(result.kind, StatementResultKind::CopyOut);
    assert_eq!(
        result.columns,
        [("id".to_owned(), Some("INTEGER".to_owned()))]
    );
    assert_eq!(result.rows, [vec![Value::from_i64(7)]]);
    connection.close()?;
    Ok(())
}

// [spec:pgorm:req:turso.capabilities/test]
#[test]
fn native_lock_accepts_a_missing_relation() -> TestResult {
    let connection = native_connection()?;
    let sql = "LOCK TABLE missing_relation IN ACCESS SHARE MODE";
    let error = native_query(&connection, sql, &[]).expect_err("lock requires transaction");
    assert_eq!(error.sqlstate, "22000");
    assert_eq!(
        error.to_string(),
        "LOCK TABLE can only be used in transaction blocks"
    );
    connection.execute("BEGIN")?;
    let result = native_query(&connection, sql, &[])?;
    assert_eq!(result.kind, StatementResultKind::Command);
    assert!(result.rows.is_empty());
    let missing = native_query(&connection, "SELECT * FROM missing_relation", &[])
        .expect_err("locked table does not exist");
    assert_eq!(missing.sqlstate, "42P01");
    connection.execute("ROLLBACK")?;
    connection.close()?;
    Ok(())
}

// [spec:pgorm:req:turso.capabilities/test]
#[test]
fn native_merge_refuses_before_target_changes() -> TestResult {
    let connection = native_connection()?;
    connection.execute("CREATE TABLE merge_target(id INTEGER PRIMARY KEY, value INTEGER); CREATE TABLE merge_source(id INTEGER PRIMARY KEY, value INTEGER); INSERT INTO merge_target VALUES(1,10); INSERT INTO merge_source VALUES(1,20),(2,30)")?;
    let error = native_query(&connection, "MERGE INTO merge_target AS t USING merge_source AS s ON t.id=s.id WHEN MATCHED THEN UPDATE SET value=s.value WHEN NOT MATCHED THEN INSERT(id,value) VALUES(s.id,s.value)", &[]).expect_err("MERGE unsupported");
    assert_eq!(error.sqlstate, "0A000");
    assert_eq!(error.to_string(), "MERGE is not supported");
    let result = native_query(
        &connection,
        "SELECT id, value FROM merge_target ORDER BY id",
        &[],
    )?;
    assert_eq!(result.rows, [vec![Value::from_i64(1), Value::from_i64(10)]]);
    connection.close()?;
    Ok(())
}

// [spec:pgorm:req:turso.capabilities/test]
#[tokio::test]
async fn driver_catalog_uses_native_type_metadata() -> TestResult {
    let database = TursoDatabase::open(":memory:").await?;
    let (client, connection) = database.connect().await?;
    assert!(client.is_turso());
    let task = tokio::spawn(connection);
    client
        .batch_execute("CREATE TABLE cap_probe(id INTEGER, label TEXT)")
        .await?;
    let rows = client.query(CLASS, &[]).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].try_get::<_, &str>(0)?, "cap_probe");
    assert_eq!(rows[0].try_get::<_, &str>(1)?, "r");
    assert!(
        rows[0]
            .columns()
            .iter()
            .all(|column| column.type_() == &Type::TEXT)
    );
    let rows = client.query(ATTRIBUTE, &[]).await?;
    assert_eq!(rows.len(), 2);
    for (row, (name, number)) in rows.iter().zip([("id", 1_i32), ("label", 2_i32)]) {
        assert_eq!(row.try_get::<_, &str>(0)?, name);
        assert_eq!(row.try_get::<_, i32>(1)?, number);
        assert_eq!(row.columns()[1].type_(), &Type::INT4);
    }
    let rows = client.query(TYPES, &[]).await?;
    assert_eq!(rows.len(), 2);
    for (row, name) in rows.iter().zip(["int4", "text"]) {
        assert_eq!(row.try_get::<_, &str>(0)?, name);
        assert_eq!(row.try_get::<_, &str>(1)?, "b");
    }
    for sql in [TABLES, COLUMNS] {
        let error = client
            .query(sql, &[])
            .await
            .expect_err("native catalog gap");
        assert_eq!(error.code(), Some(&SqlState::UNDEFINED_TABLE));
        assert!(error.source().is_some());
    }
    drop(client);
    tokio::time::timeout(Duration::from_secs(5), task).await???;
    Ok(())
}

// [spec:pgorm:req:turso.capabilities/test]
#[tokio::test]
async fn driver_preserves_native_function_refusals() -> TestResult {
    let database = TursoDatabase::open(":memory:").await?;
    let (client, connection) = database.connect().await?;
    let task = tokio::spawn(connection);
    for (sql, function) in [
        ("SELECT pg_notify('cap_topic', 'payload')", "pg_notify"),
        ("SELECT pg_advisory_xact_lock(7)", "pg_advisory_xact_lock"),
        ("SELECT to_regclass('public.cap_probe')", "to_regclass"),
    ] {
        let error = client
            .query(sql, &[])
            .await
            .expect_err("native function gap");
        assert_eq!(error.code(), Some(&SqlState::UNDEFINED_FUNCTION));
        assert_eq!(
            error.as_db_error().map(|error| error.message()),
            Some(format!("no such function: {function}").as_str())
        );
    }
    drop(client);
    tokio::time::timeout(Duration::from_secs(5), task).await???;
    Ok(())
}

// [spec:pgorm:req:turso.capabilities/test]
#[tokio::test]
async fn typed_parameters_do_not_repair_missing_functions() -> TestResult {
    let database = TursoDatabase::open(":memory:").await?;
    let (client, connection) = database.connect().await?;
    let task = tokio::spawn(connection);
    client
        .batch_execute("CREATE TABLE cap_probe(id INTEGER, label TEXT)")
        .await?;
    let missing: Option<&str> = None;
    for (sql, params) in [
        (
            "SELECT pg_notify($1, $2)",
            vec![
                (
                    &"cap_topic" as &(dyn pgorm_driver::types::ToSql + Sync),
                    Type::TEXT,
                ),
                (&"payload", Type::TEXT),
            ],
        ),
        (
            "SELECT pg_advisory_xact_lock($1)",
            vec![(
                &7_i64 as &(dyn pgorm_driver::types::ToSql + Sync),
                Type::INT8,
            )],
        ),
        (
            "SELECT to_regclass($1)",
            vec![(
                &"public.cap_probe" as &(dyn pgorm_driver::types::ToSql + Sync),
                Type::TEXT,
            )],
        ),
        (
            ADOPT,
            vec![
                (
                    &"missing_ledger" as &(dyn pgorm_driver::types::ToSql + Sync),
                    Type::TEXT,
                ),
                (&"cap_probe", Type::TEXT),
            ],
        ),
        (
            LEDGER,
            vec![
                (
                    &missing as &(dyn pgorm_driver::types::ToSql + Sync),
                    Type::TEXT,
                ),
                (&"cap_probe", Type::TEXT),
                (&missing, Type::TEXT),
            ],
        ),
        (
            CHECKSUM,
            vec![
                (
                    &missing as &(dyn pgorm_driver::types::ToSql + Sync),
                    Type::TEXT,
                ),
                (&"cap_probe", Type::TEXT),
                (&"label", Type::TEXT),
            ],
        ),
    ] {
        let untyped: Vec<_> = params.iter().map(|(value, _)| *value).collect();
        for (route, result) in [
            ("untyped", client.query(sql, &untyped).await),
            ("typed", client.query_typed(sql, &params).await),
        ] {
            let error = result.expect_err("native preparation must refuse missing objects");
            let (code, message) = if sql == CHECKSUM {
                (SqlState::UNDEFINED_TABLE, "no such table: main.columns")
            } else if sql.contains("pg_notify") {
                (SqlState::UNDEFINED_FUNCTION, "no such function: pg_notify")
            } else if sql.contains("pg_advisory_xact_lock") {
                (
                    SqlState::UNDEFINED_FUNCTION,
                    "no such function: pg_advisory_xact_lock",
                )
            } else {
                (
                    SqlState::UNDEFINED_FUNCTION,
                    "no such function: to_regclass",
                )
            };
            assert_eq!(error.code(), Some(&code), "{route} {sql}");
            assert_eq!(
                error.as_db_error().map(|error| error.message()),
                Some(message)
            );
            assert!(error.source().is_some());
        }
    }
    drop(client);
    tokio::time::timeout(Duration::from_secs(5), task).await???;
    Ok(())
}
