//! COPY inside a transaction: the scope task owning the borrowed transaction
//! drives the COPY, while the native handle holds the transaction's
//! reservation until the COPY ends.

use super::{
    State,
    scope::{Command, Tx},
    state::Pending,
};
use crate::{
    copy::{
        Columns, CopyFrom, CopyTo, Message, Piece, Unopened, drive_in, drive_out, open_in, open_out,
    },
    errors::{LifecycleError, database_error},
};
use pgorm::Error;
use pyo3::prelude::*;
use std::sync::Arc;
use tokio::sync::{OwnedMutexGuard, mpsc, oneshot};

/// A transaction reserved by a COPY or a stream in progress. Dropped without
/// `complete`, what it ran is in an unknown state, and the whole transaction
/// is discarded.
pub(crate) struct Lease {
    _busy: OwnedMutexGuard<()>,
    pending: Pending,
}

impl Lease {
    pub(super) fn new(busy: OwnedMutexGuard<()>, pending: Pending) -> Self {
        Self {
            _busy: busy,
            pending,
        }
    }

    pub(crate) fn complete(mut self) {
        self.pending.complete();
    }
}

impl State {
    /// Reserve the transaction and have its owner start `command`, answering
    /// the reservation once the server has accepted the COPY.
    async fn copy(
        self: &Arc<Self>,
        command: impl FnOnce(oneshot::Sender<Result<(), Unopened>>) -> Command,
        refusal_closes: bool,
    ) -> PyResult<Lease> {
        self.ensure_open()?;
        let busy = self.busy.clone().try_lock_owned().map_err(|_| {
            LifecycleError::new_err("transaction has an active operation or nested savepoint")
        })?;
        let (opened, open) = oneshot::channel();
        let mut pending = Pending::new(self.root.aborted.clone());
        self.commands
            .send(command(opened))
            .map_err(|_| LifecycleError::new_err("transaction owner has closed"))?;
        let result = open
            .await
            .map_err(|_| LifecycleError::new_err("transaction owner has closed"))?;
        match result {
            Ok(()) => Ok(Lease::new(busy, pending)),
            Err(unopened) => {
                // A refused COPY FROM has closed the connection under the
                // transaction, so `pending` is left to abort it; a refused
                // description has closed nothing.
                if !refusal_closes || matches!(unopened, Unopened::Undescribed(_)) {
                    pending.complete();
                }
                Err(database_error(
                    unopened.into_error(),
                    &self.connection.pool.secrets,
                ))
            }
        }
    }

    /// Wait until the root transaction's owner has let go of its connection.
    pub(crate) async fn released(&self) {
        self.root.finished.cancelled().await;
    }

    // [spec:pgorm:req:bindings.copy]
    pub(crate) async fn copy_in(
        self: &Arc<Self>,
        statement: CopyFrom,
        rows: Option<Columns>,
        messages: mpsc::Receiver<Message>,
    ) -> PyResult<(Lease, oneshot::Receiver<Result<Option<u64>, Error>>)> {
        let (reply, settled) = oneshot::channel();
        let lease = self
            .copy(
                |opened| Command::CopyIn {
                    statement,
                    rows,
                    messages,
                    opened,
                    reply,
                },
                true,
            )
            .await?;
        Ok((lease, settled))
    }

    // [spec:pgorm:req:bindings.copy]
    pub(crate) async fn copy_out(
        self: &Arc<Self>,
        statement: CopyTo,
        rows: Option<Columns>,
        pieces: mpsc::Sender<Piece>,
    ) -> PyResult<Lease> {
        self.copy(
            |opened| Command::CopyOut {
                statement,
                rows,
                pieces,
                opened,
            },
            false,
        )
        .await
    }
}

/// How a COPY FROM on the owner's transaction ended.
pub(super) enum RanIn {
    /// It never started, was refused — which closes the connection — or was
    /// abandoned.
    Lost,
    /// Its description was refused, which leaves the transaction as any
    /// failed statement does.
    Undescribed,
    Ran(Result<Option<u64>, Error>),
}

/// Run a COPY FROM on the owner's transaction.
pub(super) async fn run_in(
    tx: &mut Tx<'_, '_>,
    statement: &CopyFrom,
    rows: Option<&Columns>,
    messages: &mut mpsc::Receiver<Message>,
    opened: oneshot::Sender<Result<(), Unopened>>,
) -> RanIn {
    let started = match tx {
        Tx::Plain(tx) => open_in(&mut **tx, statement, rows).await,
        Tx::Metered(tx) => open_in(&mut **tx, statement, rows).await,
    };
    match started {
        Err(Unopened::Undescribed(error)) => {
            let _ = opened.send(Err(Unopened::Undescribed(error)));
            RanIn::Undescribed
        }
        Err(refused) => {
            let _ = opened.send(Err(refused));
            RanIn::Lost
        }
        Ok(sink) => match opened.send(Ok(())) {
            Ok(()) => RanIn::Ran(drive_in(sink, messages).await),
            Err(_) => RanIn::Lost,
        },
    }
}

/// Run a COPY TO on the owner's transaction: whether the transaction is
/// still in a known state. A refusal aborts it as any failed statement does;
/// a reader gone part way leaves the server sending.
pub(super) async fn run_out(
    tx: &mut Tx<'_, '_>,
    statement: &CopyTo,
    rows: Option<&Columns>,
    pieces: &mpsc::Sender<Piece>,
    opened: oneshot::Sender<Result<(), Unopened>>,
) -> bool {
    let started = match tx {
        Tx::Plain(tx) => open_out(&mut **tx, statement, rows).await,
        Tx::Metered(tx) => open_out(&mut **tx, statement, rows).await,
    };
    match started {
        Err(unopened) => opened.send(Err(unopened)).is_ok(),
        Ok(stream) => {
            if opened.send(Ok(())).is_err() {
                return false;
            }
            match drive_out(stream, pieces).await {
                Some(result) => pieces
                    .send(Piece::End(Box::new((result, None))))
                    .await
                    .is_ok(),
                None => false,
            }
        }
    }
}
