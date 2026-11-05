//! Transaction handles communicate with a Rust task owning the borrowed scope.
mod capabilities;
mod copy;
mod native;
mod retry;
mod scope;
mod state;
mod stream;
pub(crate) mod work;

use crate::{
    errors::{LifecycleError, database_error},
    runtime::{ConnectionState, Operation},
};
pub(crate) use capabilities::operations as capabilities;
pub(crate) use copy::Lease;
pub(crate) use native::{NativeRetrying, NativeTransaction, mode};
use pgorm::TransactionMode;
use pyo3::prelude::*;
pub(crate) use retry::begin as begin_retrying;
pub(crate) use state::State;
use state::{Finished, Pending};
use std::sync::Arc;
use tokio::sync::oneshot;

// [spec:pgorm:req:bindings.transactions+1]
pub(crate) async fn begin(
    connection: Arc<ConnectionState>,
    mode: TransactionMode,
) -> PyResult<Arc<State>> {
    let mut operation = Operation::begin(connection.clone())?;
    let (state, receiver) = State::channel(connection.clone(), None);
    let (opened, receive) = oneshot::channel();
    let mut pending = Pending::new(state.root.aborted.clone());
    let owner = state.clone();
    tokio::spawn(async move {
        let _finished = Finished(owner.root.finished.clone());
        let result = tokio::select! {
            _ = owner.root.aborted.cancelled() => None,
            _ = connection.cancelled.cancelled() => None,
            _ = connection.pool.cancelled.cancelled() => None,
            result = async {
                let connection = match operation.connection_mut() {
                    Ok(connection) => connection,
                    Err(error) => { let _ = opened.send(Err(error)); return None; }
                };
                let mut frame = scope::Frame::new(owner.clone(), receiver, opened);
                let halt = frame.halt.clone();
                let ended = halt
                    .run(connection.transaction_with(mode, async |tx| frame.serve(tx).await))
                    .await;
                match frame.end(ended) {
                    scope::Ended::Ran(completion) => Some(completion),
                    scope::Ended::Refused(opened, error) => {
                        let _ = opened.send(Err(database_error(error, &owner.connection.pool.secrets)));
                        None
                    }
                }
            } => result,
        };
        match result {
            Some(completion) => {
                if completion.healthy {
                    operation.restore();
                } else {
                    drop(operation);
                }
                completion.respond();
                owner.released.cancel();
            }
            None => drop(operation),
        }
    });
    receive
        .await
        .map_err(|_| LifecycleError::new_err("transaction owner ended during begin"))??;
    pending.complete();
    Ok(state)
}
