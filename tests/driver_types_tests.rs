//! The types of tokio-postgres, deadpool, `bytes` and `futures` that using
//! pgorm makes a caller write, each named through pgorm and nothing else.
//!
//! The live tests run in the maintenance database and create nothing there.

use std::time::Duration;

use pgorm::{
    Bytes, BytesMut, ChannelBinding, Config, ConnectionTrait, CopyInSink, CopyOutStream,
    CopyRowEncoder, CopyRowStream, DbError, Error, Hook, HookError, Host, LoadBalanceHosts,
    Notifications, PoolConfig, PoolError, PostgresError, QueueMode, Row, RowStream, Runtime, Sink,
    SqlState, SslMode, SslNegotiation, Status, Stream, TargetSessionAttrs, TimeoutType, Timeouts,
    connect_with_builder,
};

fn maintenance_config() -> Config {
    dotenvy::from_filename(".env.local").ok();
    dotenvy::from_filename(".env").ok();
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL names the test server");
    pgorm_test_support::config(&url, "postgres")
}

fn sink<S: Sink<Bytes, Error = Error>>() {}

fn stream<S: Stream>() {}

// [spec:pgorm:req:conn.driver-types/test]    the traits pgorm's sinks and streams implement
#[test]
fn sinks_and_streams_are_named_through_pgorm() {
    sink::<CopyInSink<'static>>();
    stream::<CopyOutStream<'static>>();
    stream::<CopyRowStream<'static>>();
    stream::<RowStream>();
    stream::<Notifications>();
}

// [spec:pgorm:req:conn.driver-types/test]    what Config's setters take and its getters return
#[test]
fn config_vocabulary_is_named_through_pgorm() {
    let mut config = Config::new();
    config
        .host("bakery.internal")
        .ssl_mode(SslMode::Require)
        .ssl_negotiation(SslNegotiation::Direct)
        .target_session_attrs(TargetSessionAttrs::ReadWrite)
        .channel_binding(ChannelBinding::Require)
        .load_balance_hosts(LoadBalanceHosts::Random);

    assert!(matches!(config.get_hosts(), [Host::Tcp(host)] if host == "bakery.internal"));
    assert_eq!(config.get_ssl_mode(), SslMode::Require);
    assert_eq!(config.get_ssl_negotiation(), SslNegotiation::Direct);
    assert_eq!(
        config.get_target_session_attrs(),
        TargetSessionAttrs::ReadWrite
    );
    assert_eq!(config.get_channel_binding(), ChannelBinding::Require);
    assert_eq!(config.get_load_balance_hosts(), LoadBalanceHosts::Random);
}

// [spec:pgorm:req:conn.driver-types/test]    the buffer the COPY encoder writes into
#[test]
fn copy_buffer_is_named_through_pgorm() {
    let mut data = BytesMut::new();
    CopyRowEncoder::header(&mut data);
    CopyRowEncoder::trailer(&mut data);
    assert_eq!(&data[..11], b"PGCOPY\n\xff\r\n\0");
    assert_eq!(data.len(), 21);
}

fn first_column(row: &Row) -> i32 {
    row.get(0)
}

fn sqlstate(error: &PostgresError) -> Option<&SqlState> {
    error.code()
}

fn server_error(error: &PostgresError) -> Option<&DbError> {
    error.as_db_error()
}

// [spec:pgorm:req:conn.driver-types/test]    rows, the driver's error and the SQLSTATE it reports
#[tokio::test]
async fn rows_and_driver_errors_are_named_through_pgorm() -> Result<(), Error> {
    let pool = pgorm::connect(maintenance_config());
    let db = pool.get().await?;

    let row: Row = db.query_one("SELECT 7", &[]).await?;
    assert_eq!(first_column(&row), 7);

    let rows: RowStream = db.query_raw("SELECT 1", Vec::<i32>::new()).await?;
    drop(rows);

    let missing = db
        .query_all("SELECT * FROM driver_types_no_such_table", &[])
        .await;
    let Err(Error::Postgres(error)) = missing else {
        panic!("the server refuses a table that does not exist, got {missing:?}");
    };
    assert_eq!(sqlstate(&error), Some(&SqlState::UNDEFINED_TABLE));
    let server = server_error(&error).expect("the refusal came from the server");
    assert_eq!(server.code(), &SqlState::UNDEFINED_TABLE);

    drop(db);
    pool.close();
    Ok(())
}

// [spec:pgorm:req:conn.driver-types/test]    the pool's setters' arguments, its status and its error
#[tokio::test]
async fn pool_vocabulary_is_named_through_pgorm() -> Result<(), Error> {
    let pool = connect_with_builder(maintenance_config(), |builder| {
        builder
            .config(PoolConfig {
                max_size: 1,
                timeouts: Timeouts {
                    wait: Some(Duration::from_millis(100)),
                    create: Some(Duration::from_secs(5)),
                    recycle: None,
                },
                queue_mode: QueueMode::Lifo,
            })
            .runtime(Runtime::Tokio1)
    })?;

    let held = pool.get().await?;
    let status: Status = pool.status();
    assert_eq!((status.max_size, status.size, status.available), (1, 1, 0));
    let waited = pool.get().await;
    assert!(
        matches!(
            waited,
            Err(Error::Pool(PoolError::Timeout(TimeoutType::Wait)))
        ),
        "{waited:?}"
    );
    drop(held);
    pool.close();

    let refusing = connect_with_builder(maintenance_config(), |builder| {
        builder.post_create(Hook::sync_fn(|_, _| {
            Err(HookError::message("this pool takes no connections"))
        }))
    })?;
    let refused = refusing.get().await;
    assert!(
        matches!(
            &refused,
            Err(Error::Pool(PoolError::PostCreateHook(HookError::Message(message))))
                if message == "this pool takes no connections"
        ),
        "{refused:?}"
    );
    refusing.close();
    Ok(())
}
