use crate::{BoxFuture, Connect, Unpooled};
use pgorm_driver::{Client, Config, Error, TursoDatabase};
use tokio::task::JoinHandle;

/// Experimental connection factory for one shared native Turso database.
///
/// Each connection has its own client and lifecycle task. All connections
/// share this database handle, including when it represents `:memory:`.
/// PostgreSQL connection settings passed through [`Connect`] are unused.
// [spec:pgorm:req:turso.pool]
#[derive(Debug)]
pub struct TursoConnectImpl {
    /// The database shared by every connection this factory creates.
    pub database: TursoDatabase,
}

impl Connect for TursoConnectImpl {
    fn connect(&self, _: &Config) -> BoxFuture<'_, Result<(Client, JoinHandle<()>), Error>> {
        Box::pin(async {
            let (client, connection) = self.database.connect().await?;
            let task = tokio::spawn(async move {
                if let Err(error) = connection.await {
                    tracing::warn!(target: "pgorm_pool::turso", %error, "Native connection error");
                }
            });
            Ok((client, task))
        })
    }

    // [spec:pgorm:req:conn.listen+3] native notification refusal opens no client
    fn connect_raw(&self, _: &Config) -> BoxFuture<'_, Result<Unpooled, Error>> {
        Box::pin(async { Err(Error::turso_unsupported("native notification connections")) })
    }
}
