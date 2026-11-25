use pgorm::{DatabasePool, ManagerConfig};
use pgorm_driver::{Client, Config, NoTls};
use std::{error::Error, path::PathBuf, time::Duration};

pub type Result<T = ()> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
pub const LIMIT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Postgres,
    Memory,
    File,
}

pub struct Fixture {
    base: String,
    name: String,
    directory: PathBuf,
}

impl Fixture {
    pub async fn new(label: &str) -> Result<Self> {
        dotenvy::from_filename(".env.local").ok();
        dotenvy::from_filename(".env").ok();
        let base = std::env::var("DATABASE_URL")?;
        let name = pgorm_test_support::scoped_name(label);
        let directory = std::env::temp_dir().join(format!("pgorm-oracle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory)?;
        let fixture = Self {
            base,
            name,
            directory,
        };
        pgorm_test_support::create_database(&fixture.base, &fixture.name).await?;
        Ok(fixture)
    }

    pub async fn finish(self) -> Result {
        pgorm_test_support::drop_database(&self.base, &self.name).await?;
        Ok(())
    }

    // [spec:pgorm:req:turso.compatibility] only PostgreSQL uses CREATE/DROP DATABASE
    pub async fn clients<T>(
        &self,
        operation: impl AsyncFn(Backend, &mut Client) -> Result<T>,
    ) -> Result<Vec<(Backend, T)>> {
        let mut results = Vec::new();
        for backend in [Backend::Postgres, Backend::Memory, Backend::File] {
            let (mut client, lifetime) = match backend {
                Backend::Postgres => {
                    let (client, connection) = pgorm_test_support::config(&self.base, &self.name)
                        .connect(NoTls)
                        .await?;
                    (client, tokio::spawn(connection))
                }
                Backend::Memory | Backend::File => {
                    let path = if backend == Backend::Memory {
                        PathBuf::from(":memory:")
                    } else {
                        self.directory.join("driver.db")
                    };
                    let (client, connection) = path
                        .to_str()
                        .expect("UTF-8 fixture path")
                        .parse::<Config>()?
                        .connect(NoTls)
                        .await?;
                    (client, tokio::spawn(connection))
                }
            };
            let outcome = operation(backend, &mut client).await;
            drop(client);
            let closed = tokio::time::timeout(LIMIT, lifetime).await;
            // Teardown runs even when the operation returns an error.
            let value = outcome?;
            closed???;
            results.push((backend, value));
        }
        Ok(results)
    }

    pub async fn pool(&self, backend: Backend) -> Result<DatabasePool> {
        let manager = ManagerConfig::default();
        Ok(match backend {
            Backend::Postgres => pgorm::connect_with(
                pgorm_test_support::config(&self.base, &self.name),
                NoTls,
                manager,
                |builder| builder.max_size(4),
            )?,
            Backend::Memory | Backend::File => {
                let path = if backend == Backend::Memory {
                    PathBuf::from(":memory:")
                } else {
                    self.directory.join("pool.db")
                };
                pgorm::connect_with(
                    path.to_str().expect("UTF-8 fixture path").parse()?,
                    NoTls,
                    manager,
                    |builder| builder.max_size(4),
                )?
            }
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
