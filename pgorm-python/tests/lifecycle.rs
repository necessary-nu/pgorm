use pgorm::ConnectionTrait;

fn configuration() -> Result<pgorm::Config, Box<dyn std::error::Error>> {
    Ok(std::env::var("PGORM_TEST_DSN")?.parse()?)
}

// [spec:pgorm:req:bindings.connections+3/test]
#[tokio::test]
async fn closed_pool_preserves_existing_connections() -> Result<(), Box<dyn std::error::Error>> {
    let pool = pgorm::connect_with_builder(configuration()?, |b| b.max_size(1))?;
    let connection = pool.get().await?;
    pool.close();
    assert!(pool.is_closed());
    assert!(pool.get().await.is_err());
    assert!(
        connection
            .query_one("SELECT TRUE", &[])
            .await?
            .try_get::<_, bool>(0)?
    );
    drop(connection);
    assert_eq!(pool.status().size, 0);
    Ok(())
}

// [spec:pgorm:req:bindings.connections+3/test]
#[tokio::test]
async fn discarded_connections_are_removed_instead_of_recycled()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = pgorm::connect_with_builder(configuration()?, |b| b.max_size(1))?;
    let connection = pool.get().await?;
    let first: i32 = connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .try_get(0)?;
    connection.discard();
    assert_eq!(pool.status().size, 0);
    let replacement = pool.get().await?;
    let second: i32 = replacement
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .try_get(0)?;
    assert_ne!(first, second);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let alive: bool = replacement
                .query_one(
                    "SELECT EXISTS(SELECT FROM pg_stat_activity WHERE pid = $1)",
                    &[&first],
                )
                .await?
                .try_get(0)?;
            if !alive {
                break Ok::<(), pgorm::Error>(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await??;
    pool.close();
    drop(replacement);
    Ok(())
}
