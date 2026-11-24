use crate::connection::Request;
use crate::{Error, native};
use futures_channel::mpsc;
use std::sync::Arc;

pub(crate) enum Backend {
    Postgres(mpsc::UnboundedSender<Request>),
    Turso(Arc<native::Client>),
}

impl Backend {
    pub(crate) fn postgres(&self) -> Result<&mpsc::UnboundedSender<Request>, Error> {
        match self {
            Self::Postgres(sender) => Ok(sender),
            Self::Turso(client) if client.is_closed() => Err(Error::closed()),
            Self::Turso(_) => Err(Error::turso_unsupported("PostgreSQL wire operation")),
        }
    }

    pub(crate) fn native(&self) -> Option<&Arc<native::Client>> {
        match self {
            Self::Postgres(_) => None,
            Self::Turso(client) => Some(client),
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        match self {
            Self::Postgres(sender) => sender.is_closed(),
            Self::Turso(client) => client.is_closed(),
        }
    }

    pub(crate) fn close_channel(&self) {
        match self {
            Self::Postgres(sender) => sender.close_channel(),
            Self::Turso(client) => client.shutdown(),
        }
    }
}
