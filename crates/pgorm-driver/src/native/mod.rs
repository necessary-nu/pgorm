//! In-process Turso ownership and its connection actor.
//!
//! The worker owns all mutable native statements; the ordinary driver types
//! remain the API.

use crate::Error;
use std::{fmt, path::Path, sync::Arc};

mod client;
pub(crate) mod codecs;
mod commands;
pub(crate) mod describe;
mod dispatch;
mod query;
mod rows;
mod statement_store;
mod transaction;
mod worker;
pub(crate) use rows::Rows;
pub(crate) use statement_store::Prepared;
pub(crate) use worker::{Client, Connection};

// [spec:pgorm:sem:turso.lifecycle]
#[derive(Clone)]
pub(crate) struct Database {
    inner: Arc<turso_core::Database>,
}

impl fmt::Debug for Database {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeDatabase").finish_non_exhaustive()
    }
}

impl Database {
    // [spec:pgorm:sem:turso.lifecycle] opening and catalog initialization may block
    pub(crate) async fn open(path: &Path) -> Result<Self, Error> {
        let path = path
            .to_str()
            .ok_or_else(|| {
                Error::turso_worker(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Turso database paths must be valid UTF-8",
                ))
            })?
            .to_owned();
        let inner = tokio::task::spawn_blocking(move || {
            // No custom VFS is exposed: the pinned API selects MemoryIO or
            // synchronous PlatformIO. A syscall can block this worker, never
            // a Tokio executor worker. Async VFS cancellation is a separate
            // capability, not inferred from step_with_waker's existence.
            turso_pg::open_database(
                &path,
                None,
                turso_pg::OpenFlags::default(),
                turso_pg::tursopg_database_opts(),
            )
            .map(|(_, database)| database)
            .map_err(Error::turso)
        })
        .await
        .map_err(Error::turso_worker)??;
        Ok(Self { inner })
    }

    // [spec:pgorm:sem:turso.lifecycle] each actor connects to this shared database
    pub(crate) async fn connect(&self) -> Result<(Arc<Client>, Connection), Error> {
        worker::connect(Arc::clone(&self.inner)).await
    }
}
