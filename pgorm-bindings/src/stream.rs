//! Row streams: pgorm's `query_raw` row stream, pulled a batch of rows at a
//! time, so the driver's bounded buffer and the socket hold back the server
//! while the language is not asking for rows, and the language crosses into
//! the runtime once per batch rather than once per row.

use std::{fmt, pin::Pin, sync::Arc};

use futures_util::{FutureExt, TryStreamExt};
use pgorm::{ConnectionMut, SqlText, Writes, pgorm_query::Value};
use pgorm_driver::{Row, RowStream, types::ToSql};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{
    Failure,
    connection::{ConnectionState, Operation, POOL_CLOSED},
    transaction::{Lease, State},
};

const STREAM_CLOSED: &str = "the stream is closed";

/// The most rows a stream's first pull takes, so its first row waits on the
/// decoding of few others. Each later pull may take twice as many as the one
/// before, up to [`MOST_ROWS`], so a program that reads a few rows and stops
/// decodes few it never reads, and one that reads many crosses into the
/// runtime once per [`MOST_ROWS`] rows.
const FIRST_ROWS: usize = 16;

/// The most rows one pull takes. Past it, the crossing a pull costs (a task,
/// two wake-ups and a turn of the language's event loop) is a small part of
/// what decoding its rows costs, and a larger batch only holds more memory.
const MOST_ROWS: usize = 256;

/// The row bytes past which a pull takes no further row, so a batch of wide
/// rows holds about this much and no more than one row beyond it.
const MOST_BYTES: usize = 1 << 20;

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

/// What a pull met after the rows it took, which the next pull reports, so
/// the rows before it reach the language first.
enum Ahead<E> {
    End,
    Failed(pgorm_driver::Error),
    Refused(E),
}

/// The open result, what it holds, and what the last pull met after its rows.
struct Active<O, E> {
    rows: Pin<Box<RowStream>>,
    hold: Hold<O>,
    ahead: Option<Ahead<E>>,
    /// The most rows the next pull takes.
    limit: usize,
}

/// A stream, which holds its connection or transaction until a pull finds
/// its end, or its close; `E` is what its rows' decode refuses with.
pub struct StreamState<O = (), E = Failure> {
    connection: Arc<ConnectionState<O>>,
    active: Arc<Mutex<Option<Active<O, E>>>>,
    cancelled: CancellationToken,
    finished: CancellationToken,
    /// Fires when the transaction a transaction's stream reads in ends.
    ended: CancellationToken,
}

impl<O, E> fmt::Debug for StreamState<O, E> {
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
async fn query_raw<T: SqlText<Kind = Writes> + Sync + ?Sized>(
    db: &impl ConnectionMut,
    sql: &T,
    params: &[&(dyn ToSql + Sync)],
) -> Result<RowStream, pgorm::Error> {
    db.query_raw(sql, params.iter().copied()).await
}

impl<O: Send + Sync + 'static, E: Send + 'static> StreamState<O, E> {
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
                ahead: None,
                limit: FIRST_ROWS,
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
    // [spec:pgorm:req:bindings.streams+2]
    pub async fn on_connection<P: ToSql + Send + Sync + 'static>(
        operation: Operation<O>,
        sql: String,
        params: Vec<P>,
        value: for<'v> fn(&'v P) -> Option<&'v Value>,
        cancel: &CancellationToken,
    ) -> Result<Arc<Self>, Failure> {
        let connection = operation.state.clone();
        let db = operation.connection()?;
        #[cfg(feature = "turso")]
        let sql = pgorm::ValueSql::mapped(&sql, &params, value);
        #[cfg(not(feature = "turso"))]
        let _ = value;
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
    // [spec:pgorm:req:bindings.streams+2]
    pub async fn in_transaction<P: ToSql + Send + Sync + 'static>(
        state: &Arc<State<O>>,
        lock: tokio::sync::OwnedMutexGuard<()>,
        sql: String,
        params: Vec<P>,
        value: for<'v> fn(&'v P) -> Option<&'v Value>,
        cancel: &CancellationToken,
    ) -> Result<Arc<Self>, Failure> {
        let pool = state.connection().pool.clone();
        let (opened, mut pending) = state
            .exchange(
                move |db| {
                    Box::pin(async move {
                        #[cfg(feature = "turso")]
                        let sql = pgorm::ValueSql::mapped(&sql, &params, value);
                        #[cfg(not(feature = "turso"))]
                        let _ = value;
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

    /// Whether a pull could give a row: the stream is neither closed nor
    /// finished, and neither its connection nor its transaction has closed.
    /// A language yields the rows a pull took only while this holds, so it
    /// never yields a row the next pull would refuse.
    pub fn pullable(&self) -> bool {
        !(self.cancelled.is_cancelled()
            || self.finished.is_cancelled()
            || self.ended.is_cancelled()
            || self.connection.is_closed())
    }

    /// The next rows, at least one, as `decode` reads each, or `None` at the
    /// end, which releases what the stream holds. A pull waits for one row,
    /// then takes the rows already received, up to its limit, without
    /// waiting for more. What it meets after them, the end, a failure or a
    /// row `decode` refuses, is the next pull's, which reports it as a pull
    /// meeting it first does. A row `decode` refuses, a failure or a pull
    /// cut off discards what the stream holds, its state unknown; one the
    /// server ended the statement with leaves it known, as the last row
    /// does: a transaction's is aborted, and its commit reports so.
    // [spec:pgorm:req:bindings.streams+2]
    pub async fn next<T>(
        &self,
        cancel: &CancellationToken,
        mut decode: impl FnMut(Row) -> Result<T, E>,
    ) -> Result<Option<Result<Vec<T>, E>>, Failure> {
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
        if let Some(ahead) = active.ahead.take() {
            return self.reached(active, ahead);
        }
        let first = tokio::select! {
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
        let row = match first {
            Ok(Some(row)) => row,
            Ok(None) => return self.reached(active, Ahead::End),
            Err(error) => return self.reached(active, Ahead::Failed(error)),
        };
        let mut bytes = row.raw_size_bytes();
        let mut rows = Vec::with_capacity(active.limit);
        match decode(row) {
            Ok(decoded) => rows.push(decoded),
            Err(refused) => return self.reached(active, Ahead::Refused(refused)),
        }
        while rows.len() < active.limit && bytes < MOST_BYTES {
            let Some(next) = active.rows.try_next().now_or_never() else {
                break;
            };
            let ahead = match next {
                Ok(Some(row)) => {
                    bytes += row.raw_size_bytes();
                    match decode(row) {
                        Ok(decoded) => {
                            rows.push(decoded);
                            continue;
                        }
                        Err(refused) => Ahead::Refused(refused),
                    }
                }
                Ok(None) => Ahead::End,
                Err(error) => Ahead::Failed(error),
            };
            active.ahead = Some(ahead);
            break;
        }
        active.limit = (active.limit * 2).min(MOST_ROWS);
        *slot = Some(active);
        Ok(Some(Ok(rows)))
    }

    /// Report what a pull met with no rows before it: the end, which
    /// releases what the stream holds, a failure, or a row's refusal.
    fn reached<T>(
        &self,
        active: Active<O, E>,
        ahead: Ahead<E>,
    ) -> Result<Option<Result<Vec<T>, E>>, Failure> {
        match ahead {
            Ahead::End => {
                active.hold.release();
                self.finished.cancel();
                Ok(None)
            }
            Ahead::Failed(error) => {
                if error.as_db_error().is_some() {
                    active.hold.release();
                    self.finished.cancel();
                }
                Err(self.connection.pool.failure(&error.into()))
            }
            Ahead::Refused(refused) => Ok(Some(Err(refused))),
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
