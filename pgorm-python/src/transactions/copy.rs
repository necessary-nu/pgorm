//! COPY inside a transaction: the scope task owning the borrowed transaction
//! drives the COPY, while the native handle holds the transaction's
//! reservation until the COPY ends.

use super::State;
use crate::{
    copy::{
        Columns, CopyFrom, CopyTo, Described, Message, Piece, Unopened, describe, drive_in,
        drive_out, open_in, open_out,
    },
    errors::{LifecycleError, Raise, database_error},
};
use pgorm::Error;
use pgorm_bindings::transaction::{Flow, Job, Lease, Tx};
use pyo3::prelude::*;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// Reserve the transaction and have its owner start the COPY `job` makes,
/// answering once the server has accepted it.
async fn copy(
    state: &Arc<State>,
    job: impl FnOnce(oneshot::Sender<Result<(), Unopened>>) -> Job,
    refusal_closes: bool,
) -> PyResult<Lease> {
    let lock = state.lock().py()?;
    let (opened, open) = oneshot::channel();
    let mut pending = state.pending();
    state.send_job(job(opened)).py()?;
    let result = open
        .await
        .map_err(|_| LifecycleError::new_err("the transaction has ended"))?;
    match result {
        Ok(()) => Ok(State::lease(lock, pending)),
        Err(unopened) => {
            // A refused COPY FROM has closed the connection under the
            // transaction, so `pending` is left to abort it; a refused
            // description has closed nothing.
            if !refusal_closes || matches!(unopened, Unopened::Undescribed(_)) {
                pending.complete();
            }
            Err(database_error(
                unopened.into_error(),
                &state.connection().pool.secrets,
            ))
        }
    }
}

/// The columns `statement` reads or writes, described by the transaction's
/// owner without starting the COPY. A description the server refuses aborts
/// the transaction, as any failed statement does.
// [spec:pgorm:req:bindings.copy-rows+2]
pub(crate) async fn describe_in(
    state: &Arc<State>,
    statement: Described,
) -> PyResult<Vec<pgorm::CopyColumn>> {
    let lock = state.lock().py()?;
    let pool = state.connection().pool.clone();
    state
        .request_on(
            lock,
            move |tx| {
                Box::pin(async move {
                    describe_on(tx, &statement)
                        .await
                        .map_err(|error| pool.failure(&error))
                })
            },
            &CancellationToken::new(),
        )
        .await
        .py()
}

/// A COPY FROM in the transaction, which holds it until the COPY ends.
// [spec:pgorm:req:bindings.copy]
pub(crate) async fn copy_in(
    state: &Arc<State>,
    statement: CopyFrom,
    rows: Option<Columns>,
    mut messages: mpsc::Receiver<Message>,
) -> PyResult<(Lease, oneshot::Receiver<Result<Option<u64>, Error>>)> {
    let (reply, settled) = oneshot::channel();
    let lease = copy(
        state,
        |opened| {
            Box::new(move |tx| {
                Box::pin(async move {
                    match run_in(tx, &statement, rows.as_ref(), &mut messages, opened).await {
                        RanIn::Lost => Flow::Lost,
                        RanIn::Undescribed => Flow::Kept,
                        RanIn::Ran(result) => match reply.send(result) {
                            Ok(()) => Flow::Kept,
                            Err(_) => Flow::Lost,
                        },
                    }
                })
            })
        },
        true,
    )
    .await?;
    Ok((lease, settled))
}

/// A COPY TO in the transaction, which holds it until the stream ends.
// [spec:pgorm:req:bindings.copy]
pub(crate) async fn copy_out(
    state: &Arc<State>,
    statement: CopyTo,
    rows: Option<Columns>,
    pieces: mpsc::Sender<Piece>,
) -> PyResult<Lease> {
    copy(
        state,
        |opened| {
            Box::new(move |tx| {
                Box::pin(async move {
                    match run_out(tx, &statement, rows.as_ref(), &pieces, opened).await {
                        true => Flow::Kept,
                        false => Flow::Lost,
                    }
                })
            })
        },
        false,
    )
    .await
}

/// How a COPY FROM on the owner's transaction ended.
enum RanIn {
    /// It never started, was refused — which closes the connection — or was
    /// abandoned.
    Lost,
    /// Its description was refused, which leaves the transaction as any
    /// failed statement does.
    Undescribed,
    Ran(Result<Option<u64>, Error>),
}

/// The columns `statement` reads or writes, described on the owner's
/// transaction without starting the COPY.
async fn describe_on(
    tx: &mut Tx<'_, '_>,
    statement: &Described,
) -> Result<Vec<pgorm::CopyColumn>, Error> {
    match tx {
        Tx::Plain(tx) => describe(&mut **tx, statement).await,
        Tx::Metered(tx) => describe(&mut **tx, statement).await,
    }
}

/// Run a COPY FROM on the owner's transaction.
async fn run_in(
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
async fn run_out(
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
