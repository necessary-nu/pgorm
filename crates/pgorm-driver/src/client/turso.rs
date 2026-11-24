use super::{Client, InnerClient};
use crate::config::{SslMode, SslNegotiation};
use crate::native_client::Backend;
use crate::{Error, native};
use std::sync::Arc;

impl InnerClient {
    pub(crate) fn native(&self) -> Option<&Arc<native::Client>> {
        self.sender.native()
    }

    pub(crate) fn require_postgres(&self, operation: &'static str) -> Result<(), Error> {
        match &self.sender {
            Backend::Postgres(_) => Ok(()),
            Backend::Turso(client) if client.is_closed() => Err(Error::closed()),
            Backend::Turso(_) => Err(Error::turso_unsupported(operation)),
        }
    }
}

impl Client {
    /// Whether this client executes against an embedded Turso database.
    ///
    /// This records connection construction and remains true after closing.
    pub fn is_turso(&self) -> bool {
        self.inner.native().is_some()
    }

    pub(crate) fn from_turso(client: Arc<native::Client>) -> Client {
        Client {
            inner: Arc::new(InnerClient {
                sender: Backend::Turso(client),
                cached_typeinfo: Default::default(),
                buffer: Default::default(),
            }),
            #[cfg(feature = "runtime")]
            socket_config: None,
            ssl_mode: SslMode::Disable,
            ssl_negotiation: SslNegotiation::Postgres,
            process_id: 0,
            secret_key: 0,
        }
    }
}
