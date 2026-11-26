use pgorm_driver::Config;
use std::{error::Error as StdError, path::Path};

type Result<T = ()> = std::result::Result<T, Box<dyn StdError + Send + Sync>>;

// [spec:pgorm:req:turso.pool+1/test]
#[test]
fn paths_are_lazy_and_preserve_backend_identity() -> Result {
    for path in [
        ":memory:",
        "app.db",
        "folder/app.db",
        "./host",
        "../app.db",
        "/tmp/app.db",
        "./a=b.db",
        "folder/a=b.db",
        "a database.db",
        "C:\\data\\app.db",
    ] {
        let config: Config = path.parse()?;
        assert!(config.is_turso(), "{path}");
        assert_eq!(config.get_turso_path(), Some(Path::new(path)));
        assert_eq!(config, config.clone());
    }
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("unopened.db");
    let config: Config = path.to_str().ok_or("test path is not UTF-8")?.parse()?;
    assert!(config.is_turso());
    assert!(!path.exists());
    assert!("app\0.db".parse::<Config>().is_err());
    Ok(())
}

// [spec:pgorm:req:turso.pool+1/test]
#[test]
fn postgres_targets_keep_their_parser_errors() -> Result {
    for input in [
        "",
        "host=localhost user=postgres",
        "host=/tmp dbname=test",
        "postgres://localhost/test",
        "postgres://%zz/test",
        "postgresql:///test?host=/tmp",
    ] {
        let config: Config = input.parse()?;
        assert!(!config.is_turso(), "{input}");
        assert!(config.get_turso_path().is_none());
    }
    for input in [
        "host",
        "host localhost",
        "unknown=value",
        "host='unterminated",
        "postgres:/bad-url",
        "postgresql://localhost:bad/test",
        "POSTGRESQL://localhost/test",
        "other://host/database",
    ] {
        assert!(input.parse::<Config>().is_err(), "{input}");
    }
    Ok(())
}

// [spec:pgorm:req:turso.pool+1/test]
#[cfg(feature = "runtime")]
#[tokio::test]
async fn native_options_refuse_before_opening() -> Result {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("options.db");
    let mut config: Config = path.to_str().ok_or("test path is not UTF-8")?.parse()?;
    config.host("localhost");
    let error = config
        .connect(pgorm_driver::NoTls)
        .await
        .err()
        .ok_or("mixed native/network configuration succeeded")?;
    assert!(error.to_string().contains("PostgreSQL connection options"));
    assert!(error.code().is_none());
    assert!(!path.exists());
    Ok(())
}

// [spec:pgorm:req:turso.pool+1/test]
#[cfg(all(feature = "runtime", not(feature = "turso")))]
#[tokio::test]
async fn disabled_native_feature_does_not_open_files() -> Result {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("disabled.db");
    let config: Config = path.to_str().ok_or("test path is not UTF-8")?.parse()?;
    let error = config
        .connect(pgorm_driver::NoTls)
        .await
        .err()
        .ok_or("disabled backend connected")?;
    assert!(error.to_string().contains("`turso` Cargo feature"));
    assert!(error.code().is_none());
    assert!(!path.exists());
    Ok(())
}

#[cfg(feature = "turso")]
mod native {
    use super::*;
    use pgorm_driver::{Client, Connection, NoTls, Socket, error::SqlState, tls::NoTlsStream};
    use std::time::Duration;
    use tokio::{task::JoinHandle, time::timeout};

    const LIMIT: Duration = Duration::from_secs(5);

    async fn close(
        client: Client,
        lifetime: JoinHandle<std::result::Result<(), pgorm_driver::Error>>,
    ) -> Result {
        drop(client);
        timeout(LIMIT, lifetime).await???;
        Ok(())
    }

    // [spec:pgorm:req:turso.pool+1/test]
    #[tokio::test]
    async fn retained_config_shares_memory_and_clones_isolate() -> Result {
        let config: Config = ":memory:".parse()?;
        let (first, second) = tokio::join!(config.connect(NoTls), config.connect(NoTls));
        let (first, lifetime) = first?;
        let first_task = tokio::spawn(lifetime);
        let (second, lifetime) = second?;
        let second_task = tokio::spawn(lifetime);
        first
            .batch_execute("CREATE TABLE shared(id integer); INSERT INTO shared VALUES(7)")
            .await?;
        assert_eq!(
            second
                .query_one("SELECT id FROM shared", &[])
                .await?
                .try_get::<_, i32>(0)?,
            7
        );
        let cloned = config.clone();
        assert_eq!(config, cloned);
        let (independent, lifetime) = cloned.connect(NoTls).await?;
        let independent_task = tokio::spawn(lifetime);
        let error = independent
            .query("SELECT id FROM shared", &[])
            .await
            .expect_err("cloned memory must be isolated");
        assert_eq!(error.code(), Some(&SqlState::UNDEFINED_TABLE));
        close(first, first_task).await?;
        close(second, second_task).await?;
        let (later, lifetime) = config.connect(NoTls).await?;
        let later_task = tokio::spawn(lifetime);
        assert_eq!(
            later
                .query_one("SELECT id FROM shared", &[])
                .await?
                .try_get::<_, i32>(0)?,
            7
        );
        close(later, later_task).await?;
        close(independent, independent_task).await
    }

    // [spec:pgorm:req:turso.pool+1/test]
    #[tokio::test]
    async fn existing_connect_returns_common_connection_type() -> Result {
        let (client, mut connection): (Client, Connection<Socket, NoTlsStream>) =
            pgorm_driver::connect(":memory:", NoTls).await?;
        assert!(connection.parameter("server_version").is_none());
        let error = futures_util::future::poll_fn(|cx| connection.poll_message(cx))
            .await
            .ok_or("native messages ended without refusal")?
            .err()
            .ok_or("native messages fabricated an event")?;
        assert!(error.to_string().contains("native asynchronous messages"));
        let lifetime = tokio::spawn(connection);
        assert_eq!(
            client
                .query_one("SELECT 1", &[])
                .await?
                .try_get::<_, i32>(0)?,
            1
        );
        close(client, lifetime).await
    }

    // [spec:pgorm:req:turso.pool+1/test]
    #[tokio::test]
    async fn common_connection_drop_shuts_down_native_client() -> Result {
        let config: Config = ":memory:".parse()?;
        let (client, connection) = config.connect(NoTls).await?;
        drop(connection);
        let error = timeout(LIMIT, client.check_connection())
            .await?
            .expect_err("dropped lifecycle closes client");
        assert!(error.is_closed());
        Ok(())
    }

    // [spec:pgorm:req:turso.pool+1/test]
    #[tokio::test]
    async fn path_connect_persists_across_config_owners() -> Result {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("persisted.db");
        let text = path.to_str().ok_or("test path is not UTF-8")?;
        let config: Config = text.parse()?;
        assert!(!path.exists());
        let (client, connection) = config.connect(NoTls).await?;
        let lifetime = tokio::spawn(connection);
        client
            .batch_execute("CREATE TABLE persisted(id integer); INSERT INTO persisted VALUES(23)")
            .await?;
        close(client, lifetime).await?;
        drop(config);
        let (client, connection) = pgorm_driver::connect(text, NoTls).await?;
        let lifetime = tokio::spawn(connection);
        assert_eq!(
            client
                .query_one("SELECT id FROM persisted", &[])
                .await?
                .try_get::<_, i32>(0)?,
            23
        );
        close(client, lifetime).await
    }
}

// [spec:pgorm:req:read-only.turso/test]    a native file is marked read-only, which the configuration's equality and clone keep; `:memory:` and a PostgreSQL target are refused before anything opens
#[test]
fn read_only_marks_files_and_refuses_memory() -> Result {
    let mut config: Config = "./shop.db".parse()?;
    assert!(!config.get_turso_read_only());
    config.turso_read_only()?;
    assert!(config.get_turso_read_only());
    assert_eq!(config.clone(), config);
    assert_ne!(config, "./shop.db".parse::<Config>()?);
    assert!(format!("{config:?}").contains("turso_read_only: true"));

    let mut memory: Config = ":memory:".parse()?;
    assert_eq!(
        memory
            .turso_read_only()
            .map(|_| ())
            .unwrap_err()
            .to_string(),
        "native Turso does not support a read-only `:memory:` database"
    );
    assert!(!memory.get_turso_read_only());

    let mut postgres: Config = "postgres://localhost/shop".parse()?;
    assert!(postgres.turso_read_only().is_err());
    assert!(!postgres.get_turso_read_only());
    Ok(())
}
