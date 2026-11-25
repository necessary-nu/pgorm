//! Native refusals must precede PostgreSQL preparation and parameter encoding.
#![cfg(feature = "turso")]

use bytes::{Bytes, BytesMut};
use pgorm_driver::{
    Error, NoTls, TursoDatabase,
    types::{IsNull, ToSql, Type},
};
use std::{error::Error as StdError, time::Duration};

#[derive(Debug)]
struct MustNotEncode;

impl ToSql for MustNotEncode {
    fn to_sql(
        &self,
        _: &Type,
        _: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn StdError + Send + Sync>> {
        panic!("an unsupported native operation reached the PostgreSQL encoder");
    }

    fn accepts(_: &Type) -> bool {
        true
    }

    pgorm_driver::types::to_sql_checked!();
}

fn unsupported<T>(result: Result<T, Error>, operation: &str) {
    let error = result.err().expect("operation is unsupported");
    assert!(!error.is_closed(), "a live native connection is not closed");
    assert!(
        error.as_db_error().is_none(),
        "a driver refusal is not an engine error"
    );
    assert!(error.to_string().contains(operation), "{error}");
}

// [spec:pgorm:sem:turso.lifecycle/test]
#[tokio::test]
async fn simple_query_refusal_has_no_side_effects() {
    let database = TursoDatabase::open(":memory:").await.unwrap();
    let (client, connection) = database.connect().await.unwrap();
    let task = tokio::spawn(connection);
    unsupported(
        client
            .simple_query("CREATE TABLE refused(id integer)")
            .await,
        "simple_query_raw",
    );
    client
        .batch_execute("CREATE TABLE refused(id integer)")
        .await
        .unwrap();
    client.check_connection().await.unwrap();
    drop(client);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

// [spec:pgorm:sem:turso.lifecycle/test]
#[tokio::test]
async fn wire_only_operations_refuse_without_closing_native_client() {
    let database = TursoDatabase::open(":memory:").await.unwrap();
    let (mut client, connection) = database.connect().await.unwrap();
    let task = tokio::spawn(connection);
    unsupported(
        client.copy_in::<_, Bytes>("COPY absent FROM STDIN").await,
        "COPY FROM STDIN",
    );
    unsupported(
        client.copy_out("COPY absent TO STDOUT").await,
        "COPY TO STDOUT",
    );
    let (stream, _peer) = tokio::io::duplex(64);
    unsupported(
        client.cancel_token().cancel_query_raw(stream, NoTls).await,
        "cancellation through a network stream",
    );
    {
        let transaction = client.transaction().await.unwrap();
        unsupported(
            transaction.bind("SELECT $1", &[&MustNotEncode]).await,
            "portals",
        );
        transaction.rollback().await.unwrap();
    }
    client.check_connection().await.unwrap();
    drop(client);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

// [spec:pgorm:req:turso.capabilities/test]
#[tokio::test]
async fn generic_copy_routes_refuse_before_execution() {
    let database = TursoDatabase::open(":memory:").await.unwrap();
    let (client, connection) = database.connect().await.unwrap();
    let task = tokio::spawn(connection);
    client
        .batch_execute("CREATE TABLE copied(id integer); INSERT INTO copied VALUES(1),(2)")
        .await
        .unwrap();
    let sql = "COPY copied TO STDOUT";
    unsupported(client.prepare(sql).await, "COPY TO STDOUT");
    unsupported(client.prepare_typed(sql, &[]).await, "COPY TO STDOUT");
    unsupported(client.query(sql, &[]).await, "COPY TO STDOUT");
    unsupported(client.query(sql, &[&MustNotEncode]).await, "COPY TO STDOUT");
    unsupported(client.execute(sql, &[]).await, "COPY TO STDOUT");
    unsupported(
        client.query_raw(sql, std::iter::empty::<i32>()).await,
        "COPY TO STDOUT",
    );
    unsupported(client.query_typed(sql, &[]).await, "COPY TO STDOUT");
    unsupported(
        client
            .batch_execute("COPY copied TO STDOUT; INSERT INTO copied VALUES(3)")
            .await,
        "COPY TO STDOUT",
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM copied", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        2
    );
    client.check_connection().await.unwrap();
    drop(client);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
