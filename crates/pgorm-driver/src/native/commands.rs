use super::{
    rows::Output,
    statement_store::{Prepared, PreparedLease},
    worker::Operation,
};
use crate::{Error, Statement, types::Type};
use std::sync::Arc;
use tokio::sync::oneshot;

pub(super) enum Command {
    Batch {
        sql: String,
        operation: Arc<Operation>,
        reply: oneshot::Sender<Result<(), Error>>,
    },
    Prepare {
        sql: String,
        types: Vec<Type>,
        operation: Arc<Operation>,
        reply: oneshot::Sender<Result<(Prepared, PreparedLease), Error>>,
    },
    Query {
        id: u64,
        statement: Statement,
        params: Vec<turso_core::Value>,
        operation: Arc<Operation>,
        ready: oneshot::Sender<Result<(), Error>>,
        output: Output,
    },
    Execute {
        id: u64,
        params: Vec<turso_core::Value>,
        operation: Arc<Operation>,
        reply: oneshot::Sender<Result<u64, Error>>,
        // Keep the public statement alive until the operation completes.
        _statement: Statement,
    },
}
impl Command {
    pub(super) fn operation(&self) -> &Arc<Operation> {
        match self {
            Self::Batch { operation, .. }
            | Self::Prepare { operation, .. }
            | Self::Query { operation, .. }
            | Self::Execute { operation, .. } => operation,
        }
    }

    pub(super) fn close(self) {
        match self {
            Self::Batch { reply, .. } => {
                let _ = reply.send(Err(Error::closed()));
            }
            Self::Prepare { reply, .. } => {
                let _ = reply.send(Err(Error::closed()));
            }
            Self::Query { ready, output, .. } => {
                let _ = ready.send(Err(Error::closed()));
                output.finish(Err(Error::closed()));
            }
            Self::Execute { reply, .. } => {
                let _ = reply.send(Err(Error::closed()));
            }
        }
    }
}
