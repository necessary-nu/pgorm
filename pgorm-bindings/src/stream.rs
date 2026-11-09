//! Row streams: pgorm's `query_raw` row stream, pulled a row at a time, so
//! the driver's bounded buffer and the socket hold back the server while the
//! language is not asking for rows.

use std::{fmt, pin::Pin, sync::Arc};

use futures_util::TryStreamExt;
use pgorm::ConnectionTrait;
use tokio::sync::Mutex;
use tokio_postgres::{Row, RowStream, types::ToSql};
use tokio_util::sync::CancellationToken;

use crate::{
    Failure,
    connection::{ConnectionState, Operation, POOL_CLOSED},
    transaction::{Lease, State},
};

const STREAM_CLOSED: &str = "the stream is closed";

/// What a stream holds while it reads: its connection, taken out of its
/// slot, or its transaction, reserved. Released at the last row; dropped
/// before it, the connection is discarded, and the transaction with it.
enum Hold<O> {
    Connection(Box<Operation<O>>),
    Transaction(Lease),
}

impl<O: Send + Sync + 'static> Hold<O> {
    fn release(self) {
        match self {
            Self::Connection(operation) => (*operation).restore(),
            Self::Transaction(lease) => lease.complete(),
        }
    }
}

/// The open result and what it holds.
struct Active<O> {
    rows: Pin<Box<RowStream>>,
    hold: Hold<O>,
}

/// A stream, which holds its connection or transaction until its last row
/// or its close.
pub struct StreamState<O = ()> {
    connection: Arc<ConnectionState<O>>,
    active: Arc<Mutex<Option<Active<O>>>>,
    cancelled: CancellationToken,
    finished: CancellationToken,
    /// Fires when the transaction a transaction's stream reads in ends.
    ended: CancellationToken,
}

impl<O> fmt::Debug for StreamState<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamState")
            .field(
                "closed",
                &(self.cancelled.is_cancelled() || self.finished.is_cancelled()),
            )
            .finish_non_exhaustive()
    }
}

/// `sql`'s rows on `db`, its parameters bound.
async fn query_raw(
    db: &impl ConnectionTrait,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<RowStream, pgorm::Error> {
    db.query_raw(sql, params.iter().copied()).await
}

impl<O: Send + Sync + 'static> StreamState<O> {
    fn held(
        connection: Arc<ConnectionState<O>>,
        rows: RowStream,
        hold: Hold<O>,
        ended: CancellationToken,
    ) -> Arc<Self> {
        let state = Arc::new(Self {
            connection,
            active: Arc::new(Mutex::new(Some(Active {
                rows: Box::pin(rows),
                hold,
            }))),
            cancelled: CancellationToken::new(),
            finished: CancellationToken::new(),
            ended,
        });
        tokio::spawn(state.clone().watch());
        state
    }

    /// `sql`'s rows on the connection `operation` took, which the stream
    /// holds until its last row.
    // [spec:pgorm:req:bindings.streams+1]
    pub async fn on_connection<P: ToSql + Send + Sync + 'static>(
        operation: Operation<O>,
        sql: String,
        params: Vec<P>,
        cancel: &CancellationToken,
    ) -> Result<Arc<Self>, Failure> {
        let connection = operation.state.clone();
        let db = operation.connection()?;
        let bound: Vec<&(dyn ToSql + Sync)> = params
            .iter()
            .map(|param| param as &(dyn ToSql + Sync))
            .collect();
        let rows = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(Failure::Cancelled),
            () = connection.closing() => {
                connection.ensure_open()?;
                return Err(Failure::Lifecycle(POOL_CLOSED.to_owned()));
            }
            rows = query_raw(&db, &sql, &bound) => {
                rows.map_err(|error| connection.pool.failure(&error))?
            }
        };
        Ok(Self::held(
            connection,
            rows,
            Hold::Connection(Box::new(operation)),
            CancellationToken::new(),
        ))
    }

    /// `sql`'s rows in the transaction `state`, the stream holding the lock
    /// taken as the language opened it until its last row. A statement the
    /// server refuses leaves the transaction as any failed statement does.
    // [spec:pgorm:req:bindings.streams+1]
    pub async fn in_transaction<P: ToSql + Send + Sync + 'static>(
        state: &Arc<State<O>>,
        lock: tokio::sync::OwnedMutexGuard<()>,
        sql: String,
        params: Vec<P>,
        cancel: &CancellationToken,
    ) -> Result<Arc<Self>, Failure> {
        let pool = state.connection().pool.clone();
        let (opened, mut pending) = state
            .exchange(
                move |db| {
                    Box::pin(async move {
                        let bound: Vec<&(dyn ToSql + Sync)> = params
                            .iter()
                            .map(|param| param as &(dyn ToSql + Sync))
                            .collect();
                        query_raw(&db, &sql, &bound)
                            .await
                            .map_err(|error| pool.failure(&error))
                    })
                },
                cancel,
            )
            .await?;
        match opened {
            Ok(rows) => Ok(Self::held(
                state.connection().clone(),
                rows,
                Hold::Transaction(State::<O>::lease(lock, pending)),
                state.ended(),
            )),
            Err(failure) => {
                pending.complete();
                Err(failure)
            }
        }
    }

    /// Whether the stream has closed or reached its end.
    pub fn closed(&self) -> bool {
        self.cancelled.is_cancelled() || self.finished.is_cancelled()
    }

    /// Release an idle stream's connection once the stream, its connection
    /// or its pool closes, without waiting for the language's next pull.
    async fn watch(self: Arc<Self>) {
        tokio::select! {
            () = self.finished.cancelled() => return,
            () = self.cancelled.cancelled() => {}
            () = self.connection.closing() => {}
            () = self.ended.cancelled() => {}
        }
        drop(self.active.lock().await.take());
        self.finished.cancel();
    }

    /// The next row, as `decode` reads it, or `None` at the end, which
    /// releases what the stream holds. A row `decode` refuses, a failure or
    /// a pull cut off discards what the stream holds, its state unknown; one
    /// the server ended the statement with leaves it known, as the last row
    /// does: a transaction's is aborted, and its commit reports so.
    // [spec:pgorm:req:bindings.streams+1]
    pub async fn next<T, E>(
        &self,
        cancel: &CancellationToken,
        decode: impl FnOnce(Row) -> Result<T, E>,
    ) -> Result<Option<Result<T, E>>, Failure> {
        self.connection.ensure_open()?;
        if self.cancelled.is_cancelled() {
            return Err(Failure::Lifecycle(STREAM_CLOSED.to_owned()));
        }
        if self.finished.is_cancelled() {
            return Ok(None);
        }
        let mut slot = self.active.clone().try_lock_owned().map_err(|_| {
            Failure::Lifecycle("the stream is busy: one row is pulled at a time".to_owned())
        })?;
        let Some(mut active) = slot.take() else {
            return Ok(None);
        };
        let row = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(Failure::Cancelled),
            () = self.cancelled.cancelled() => {
                return Err(Failure::Lifecycle(STREAM_CLOSED.to_owned()));
            }
            () = self.connection.closing() => {
                self.connection.ensure_open()?;
                return Err(Failure::Lifecycle(POOL_CLOSED.to_owned()));
            }
            () = self.ended.cancelled() => {
                return Err(Failure::Lifecycle("the transaction is closed".to_owned()));
            }
            row = active.rows.try_next() => row,
        };
        match row {
            Ok(Some(row)) => {
                let decoded = decode(row);
                if decoded.is_ok() {
                    *slot = Some(active);
                }
                Ok(Some(decoded))
            }
            Ok(None) => {
                active.hold.release();
                self.finished.cancel();
                Ok(None)
            }
            Err(error) => {
                if error.as_db_error().is_some() {
                    active.hold.release();
                    self.finished.cancel();
                }
                Err(self.connection.pool.failure(&error.into()))
            }
        }
    }

    /// End the stream at once, discarding what it holds if rows remained,
    /// unless a pull holds it, which sees the close.
    pub fn close_now(&self) {
        self.cancelled.cancel();
        if let Ok(mut slot) = self.active.try_lock() {
            drop(slot.take());
        }
    }

    /// End the stream, and finish once what it held is released.
    pub async fn close(&self) {
        self.cancelled.cancel();
        drop(self.active.lock().await.take());
        self.finished.cancel();
    }

    /// The handle over the stream was dropped: what it holds is released as
    /// a backstop.
    pub fn abandon(&self) {
        self.cancelled.cancel();
    }

    /// The connection the stream reads on.
    pub fn connection(&self) -> &Arc<ConnectionState<O>> {
        &self.connection
    }
}
