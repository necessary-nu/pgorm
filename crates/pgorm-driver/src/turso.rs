use crate::{Client, Error, native};
use std::fmt;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

/// An experimental embedded Turso database shared by its driver connections.
///
/// Clones share the same database, including `:memory:`. Opening `:memory:`
/// again creates an independent database. Each call to [`Self::connect`]
/// creates a separate connection to this database.
// [spec:pgorm:req:turso.feature]
#[derive(Clone)]
pub struct TursoDatabase(native::Database);

impl TursoDatabase {
    /// Opens a file or an independent `:memory:` database in-process.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let database = native::Database::open(path.as_ref()).await?;
        Ok(Self(database))
    }

    /// Creates a client and its lifecycle future.
    ///
    /// Spawn or await the future for the connection's lifetime. Dropping it
    /// closes the client, even if the future has never been polled.
    pub async fn connect(&self) -> Result<(Client, TursoConnection), Error> {
        let (client, connection) = self.0.connect().await?;
        Ok((Client::from_turso(client), TursoConnection(connection)))
    }
}

impl fmt::Debug for TursoDatabase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TursoDatabase").finish_non_exhaustive()
    }
}

/// Owns the lifetime of a native connection's blocking worker.
///
/// Its output reports worker shutdown errors. Dropping it requests shutdown;
/// the worker releases native resources after any active operation cleans up.
pub struct TursoConnection(native::Connection);

impl Future for TursoConnection {
    type Output = Result<(), Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.get_mut().0).poll(cx)
    }
}

impl fmt::Debug for TursoConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TursoConnection").finish_non_exhaustive()
    }
}
