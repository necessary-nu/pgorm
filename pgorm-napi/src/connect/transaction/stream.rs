//! A statement's rows streamed inside a transaction: the frame's task opens
//! them, and the stream holds the frame's lock until its last row.

use neon::prelude::*;
use tokio::sync::OwnedMutexGuard;
use tokio_util::sync::CancellationToken;

use super::{Abort, Command, Failure, Lease, State, abort_argument, settle, state};

impl State {
    /// Open a statement's rows in this frame, the stream holding the lock
    /// taken as JavaScript opened it until its last row. A statement the
    /// server refuses leaves the transaction as any failed statement does.
    // [spec:pgorm:req:bindings.streams+1]
    async fn stream(
        &self,
        lock: OwnedMutexGuard<()>,
        sql: String,
        params: Vec<crate::params::Param>,
        abort: &Abort,
    ) -> Result<(Lease, tokio_postgres::RowStream), Failure> {
        let (opened, mut pending) = self
            .exchange(
                |opened| Command::Stream {
                    sql,
                    params,
                    opened,
                },
                abort,
            )
            .await?;
        match opened {
            Ok(rows) => Ok((
                Lease {
                    _lock: lock,
                    pending,
                },
                rows,
            )),
            Err(failure) => {
                pending.complete();
                Err(failure)
            }
        }
    }

    /// A token that fires once this frame has ended, by any route.
    fn ended(&self) -> CancellationToken {
        let ended = CancellationToken::new();
        let fire = ended.clone();
        let (aborted, finished, abandoned, own) = (
            self.root.aborted.clone(),
            self.root.finished.clone(),
            self.abandoned.clone(),
            self.finished.clone(),
        );
        tokio::spawn(async move {
            tokio::select! {
                () = aborted.cancelled() => {}
                () = finished.cancelled() => {}
                () = abandoned.cancelled() => {}
                () = own.cancelled() => {}
            }
            fire.cancel();
        });
        ended
    }
}

/// `transactionStream(transaction, sql, params, abort, decoder?)`: a stream
/// of the statement's rows in the transaction, which holds its lock until the
/// last row; closed before it, the transaction is discarded.
// [spec:pgorm:req:bindings.streams+1]
pub(super) fn transaction_stream(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let (sql, params) = crate::connect::sql_and_params(&mut cx, 1)?;
    let abort = abort_argument(&mut cx, 3)?;
    let decode = crate::connect::stream::decode_argument(&mut cx, 4);
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move {
        let (lease, rows) = state.stream(lock, sql, params, &abort).await?;
        Ok(crate::connect::stream::held(
            state.connection.clone(),
            rows,
            crate::connect::stream::Hold::Transaction(lease),
            state.ended(),
            decode,
        ))
    };
    settle::promise(&mut cx, work, |cx, stream| {
        Ok(cx
            .boxed(crate::connect::stream::StreamHandle(stream))
            .upcast())
    })
}
