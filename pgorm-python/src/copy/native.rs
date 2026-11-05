//! Native COPY handles. A COPY's sink or stream borrows its connection
//! exclusively, so the task owning the connection drives it — a spawned
//! pump for a Connection, the scope task for a Transaction — and the handle
//! talks to that task through bounded channels.

use std::{
    fmt,
    sync::{
        Arc, Mutex as StdMutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use bytes::{Bytes, BytesMut};
use pgorm::{CopyRowDecoder, CopyRowEncoder, Error, ValueHolder};
use pyo3::{
    buffer::PyBuffer,
    exceptions::PyTypeError,
    prelude::*,
    types::{PyByteArray, PyBytes, PyList, PySequence, PyString, PyTuple},
};
use pyo3_async_runtimes::tokio::future_into_py;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::{
    CopyFrom, CopyTo, PyCopyFrom, PyCopyTo,
    pump::{Columns, Message, Piece, QUEUED, Unopened, drive_in, drive_out, open_in, open_out},
};
use crate::{
    errors::{ConstructionError, DecodeError, InternalError, LifecycleError, database_error},
    execution::Target,
    results::{Field, value},
    runtime::{ConnectionState, Operation},
    transactions::{Lease, State},
    values::PyValue,
};

/// A hook's connection runs statements only, and `Target::extract` never
/// gives one: a COPY's target is a connection or a transaction.
const HOOKS_COPY_NOTHING: &str = "a COPY runs on a Connection or a Transaction";

/// The largest `bytes` a COPY TO pull joins from chunks already received.
const GATHERED: usize = 64 * 1024;

type Settled = (Result<Option<u64>, Error>, Operation);

/// How a COPY that started ended, from the task that drove it.
enum Outcome<T> {
    /// The server refused the statement before any data moved.
    Refused(Error),
    /// The server refused a COPY of rows' description, a statement of its
    /// own run before the COPY, which leaves the connection usable.
    Undescribed(Error),
    Ran(T),
    /// The handle or its opening future went away mid-copy.
    Abandoned,
}

impl<T> From<Unopened> for Outcome<T> {
    fn from(unopened: Unopened) -> Self {
        match unopened {
            Unopened::Refused(error) => Self::Refused(error),
            Unopened::Undescribed(error) => Self::Undescribed(error),
        }
    }
}

/// The rows flag a COPY was opened with, refused unless its format is
/// binary: the one whose rows pgorm writes and reads as values.
fn rows_columns(rows: bool, binary: bool) -> PyResult<Option<Columns>> {
    match (rows, binary) {
        (false, _) => Ok(None),
        (true, true) => Ok(Some(Arc::new(OnceLock::new()))),
        (true, false) => Err(PyTypeError::new_err(
            "rows travel in binary COPY alone: build the statement with the \"binary\" format",
        )),
    }
}

/// The described columns of a COPY of rows that has opened.
fn described(columns: &Columns) -> PyResult<&[pgorm::CopyColumn]> {
    columns
        .get()
        .map(Vec::as_slice)
        .ok_or_else(|| LifecycleError::new_err("COPY has not started"))
}

fn lock<T>(mutex: &StdMutex<T>) -> PyResult<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| InternalError::new_err("COPY handle lock failed"))
}

fn copy_from(statement: &Bound<'_, PyAny>) -> PyResult<CopyFrom> {
    statement
        .extract::<PyRef<'_, PyCopyFrom>>()
        .map(|statement| statement.inner.clone())
        .map_err(|_| PyTypeError::new_err("copy_in requires a CopyFrom statement"))
}

fn copy_to(statement: &Bound<'_, PyAny>) -> PyResult<CopyTo> {
    statement
        .extract::<PyRef<'_, PyCopyTo>>()
        .map(|statement| statement.inner.clone())
        .map_err(|_| PyTypeError::new_err("copy_out requires a CopyTo statement"))
}

/// One chunk of COPY FROM data, copied out of any byte buffer.
fn chunk(value: &Bound<'_, PyAny>) -> PyResult<Bytes> {
    let buffer = PyBuffer::<u8>::get(value)
        .map_err(|_| PyTypeError::new_err("COPY data must be a bytes-like object"))?;
    Ok(Bytes::from(buffer.to_vec(value.py())?))
}

/// Hand back the result of a COPY FROM and, for a Connection, its lease.
enum Settle {
    Connection(oneshot::Receiver<Settled>),
    Transaction(Lease, oneshot::Receiver<Result<Option<u64>, Error>>),
}

impl Settle {
    /// A dropped wait drops the lease with it: an interrupted finish leaves
    /// the outcome unknown, so the connection — or the transaction — is
    /// discarded rather than reused.
    async fn wait(self, connection: &ConnectionState) -> PyResult<Option<u64>> {
        let result = match self {
            Self::Connection(settled) => match settled.await {
                Ok((result, operation)) => {
                    operation.restore();
                    result
                }
                Err(_) => {
                    connection.ensure_open()?;
                    return Err(LifecycleError::new_err("COPY ended without completing"));
                }
            },
            Self::Transaction(lease, settled) => {
                let result = settled
                    .await
                    .map_err(|_| LifecycleError::new_err("transaction is closed"))?;
                lease.complete();
                result
            }
        };
        result.map_err(|error| database_error(error, &connection.pool.secrets))
    }
}

/// Drive a COPY FROM on a Connection, owning its lease. Every early return
/// drops the lease, which discards the connection.
async fn pump_in(
    state: Arc<ConnectionState>,
    mut operation: Operation,
    statement: CopyFrom,
    rows: Option<Columns>,
    mut messages: mpsc::Receiver<Message>,
    opened: oneshot::Sender<Result<(), Error>>,
    settled: oneshot::Sender<Settled>,
) {
    let mut opened = Some(opened);
    let outcome = tokio::select! {
        _ = state.cancelled.cancelled() => return,
        _ = state.pool.cancelled.cancelled() => return,
        outcome = async {
            let Ok(connection) = operation.connection_mut() else {
                return Outcome::Abandoned;
            };
            let sink = match open_in(connection, &statement, rows.as_ref()).await {
                Ok(sink) => sink,
                Err(unopened) => return unopened.into(),
            };
            match opened.take().map(|opened| opened.send(Ok(()))) {
                Some(Ok(())) => Outcome::Ran(drive_in(sink, &mut messages).await),
                _ => Outcome::Abandoned,
            }
        } => outcome,
    };
    match outcome {
        // A refused COPY FROM closes its connection, so the lease is
        // dropped before the refusal is reported.
        Outcome::Refused(error) => {
            drop(operation);
            if let Some(opened) = opened {
                let _ = opened.send(Err(error));
            }
        }
        Outcome::Undescribed(error) => {
            operation.restore();
            if let Some(opened) = opened {
                let _ = opened.send(Err(error));
            }
        }
        Outcome::Ran(result) => {
            let _ = settled.send((result, operation));
        }
        Outcome::Abandoned => {}
    }
}

/// Drive a COPY TO on a Connection, owning its lease. Every early return
/// drops the lease, which discards the connection: the server would
/// otherwise keep sending.
async fn pump_out(
    state: Arc<ConnectionState>,
    mut operation: Operation,
    statement: CopyTo,
    rows: Option<Columns>,
    pieces: mpsc::Sender<Piece>,
    opened: oneshot::Sender<Result<(), Error>>,
) {
    let mut opened = Some(opened);
    let outcome = tokio::select! {
        _ = state.cancelled.cancelled() => return,
        _ = state.pool.cancelled.cancelled() => return,
        outcome = async {
            let Ok(connection) = operation.connection_mut() else {
                return Outcome::Abandoned;
            };
            let stream = match open_out(connection, &statement, rows.as_ref()).await {
                Ok(stream) => stream,
                Err(unopened) => return unopened.into(),
            };
            match opened.take().map(|opened| opened.send(Ok(()))) {
                Some(Ok(())) => drive_out(stream, &pieces).await.map_or(Outcome::Abandoned, Outcome::Ran),
                _ => Outcome::Abandoned,
            }
        } => outcome,
    };
    match outcome {
        // A refused COPY TO, or a refused description, leaves the
        // connection idle, as any failed statement does; it is restored
        // before the refusal is reported.
        Outcome::Refused(error) | Outcome::Undescribed(error) => {
            operation.restore();
            if let Some(opened) = opened {
                let _ = opened.send(Err(error));
            }
        }
        Outcome::Ran(result) => {
            let _ = pieces
                .send(Piece::End(Box::new((result, Some(operation)))))
                .await;
        }
        Outcome::Abandoned => {}
    }
}

/// Start a COPY FROM on a Connection or Transaction and answer the handle
/// its data is written through.
// [spec:pgorm:req:bindings.copy]
#[pyfunction]
#[pyo3(name = "_copy_in", signature = (target, statement, rows = false))]
pub(crate) fn copy_in<'py>(
    py: Python<'py>,
    target: &Bound<'_, PyAny>,
    statement: &Bound<'_, PyAny>,
    rows: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let target = Target::extract(py, target)?;
    let statement = copy_from(statement)?;
    let columns = rows_columns(rows, matches!(statement, CopyFrom::Binary(_)))?;
    future_into_py(py, async move {
        let (messages, receiver) = mpsc::channel(QUEUED);
        let (connection, settle) = match target {
            Target::Hook(_) => return Err(PyTypeError::new_err(HOOKS_COPY_NOTHING)),
            Target::Connection(state) => {
                let operation = Operation::begin(state.clone())?;
                let (opened, open) = oneshot::channel();
                let (settled, settle) = oneshot::channel();
                tokio::spawn(pump_in(
                    state.clone(),
                    operation,
                    statement,
                    columns.clone(),
                    receiver,
                    opened,
                    settled,
                ));
                match open.await {
                    Ok(Ok(())) => (state, Settle::Connection(settle)),
                    Ok(Err(error)) => return Err(database_error(error, &state.pool.secrets)),
                    Err(_) => {
                        state.ensure_open()?;
                        return Err(LifecycleError::new_err("COPY ended before it started"));
                    }
                }
            }
            Target::Transaction(state) => {
                let (lease, settle) = state.copy_in(statement, columns.clone(), receiver).await?;
                (state.connection.clone(), Settle::Transaction(lease, settle))
            }
        };
        Ok(NativeCopyIn {
            connection,
            messages: StdMutex::new(Some(messages)),
            settle: StdMutex::new(Some(settle)),
            rows: columns.map(|columns| RowsIn {
                columns,
                headed: AtomicBool::new(false),
            }),
        })
    })
}

/// Start a COPY TO on a Connection or Transaction and answer the stream its
/// data is read from.
// [spec:pgorm:req:bindings.copy]
#[pyfunction]
#[pyo3(name = "_copy_out", signature = (target, statement, rows = false))]
pub(crate) fn copy_out<'py>(
    py: Python<'py>,
    target: &Bound<'_, PyAny>,
    statement: &Bound<'_, PyAny>,
    rows: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let target = Target::extract(py, target)?;
    let statement = copy_to(statement)?;
    let columns = rows_columns(rows, matches!(statement, CopyTo::Binary(_)))?;
    future_into_py(py, async move {
        let (pieces, receiver) = mpsc::channel(QUEUED);
        let (connection, transaction, holder) = match target {
            Target::Hook(_) => return Err(PyTypeError::new_err(HOOKS_COPY_NOTHING)),
            Target::Connection(state) => {
                let operation = Operation::begin(state.clone())?;
                let (opened, open) = oneshot::channel();
                let stopped = CancellationToken::new();
                let pump = pump_out(
                    state.clone(),
                    operation,
                    statement,
                    columns.clone(),
                    pieces,
                    opened,
                );
                let done = stopped.clone();
                tokio::spawn(async move {
                    pump.await;
                    done.cancel();
                });
                match open.await {
                    Ok(Ok(())) => (state, None, Holder::Connection(stopped)),
                    Ok(Err(error)) => return Err(database_error(error, &state.pool.secrets)),
                    Err(_) => {
                        state.ensure_open()?;
                        return Err(LifecycleError::new_err("COPY ended before it started"));
                    }
                }
            }
            Target::Transaction(state) => {
                let lease = state.copy_out(statement, columns.clone(), pieces).await?;
                (
                    state.connection.clone(),
                    Some(state.clone()),
                    Holder::Transaction(lease, state),
                )
            }
        };
        Ok(NativeCopyOut {
            state: Arc::new(OutState {
                connection,
                transaction,
                active: Arc::new(Mutex::new(Some(Active {
                    pieces: receiver,
                    stashed: None,
                    holder,
                }))),
                closing: CancellationToken::new(),
                finished: CancellationToken::new(),
                rows: columns.map(|columns| RowsOut {
                    columns,
                    decoder: StdMutex::new(None),
                }),
            }),
        })
    })
}

// [spec:pgorm:req:bindings.copy]
// [spec:pgorm:req:bindings.cancellation]
/// The data side of a COPY FROM: `write` each chunk, then `finish`, or
/// `abort` to send `CopyFail`.
#[pyclass(name = "_CopyIn", module = "pgorm._native", frozen)]
pub(crate) struct NativeCopyIn {
    connection: Arc<ConnectionState>,
    messages: StdMutex<Option<mpsc::Sender<Message>>>,
    settle: StdMutex<Option<Settle>>,
    /// Set for `copy_in_rows`, whose rows are encoded as they are written.
    rows: Option<RowsIn>,
}

/// A COPY of rows' writing side: the columns described before it started,
/// and whether the format's header has gone out.
struct RowsIn {
    columns: Columns,
    headed: AtomicBool,
}

impl RowsIn {
    fn head(&self, out: &mut BytesMut) {
        if !self.headed.swap(true, Ordering::AcqRel) {
            CopyRowEncoder::header(out);
        }
    }
}

/// The message a refusal of pgorm's carries, without the variant's prefix.
fn refusal(error: Error) -> String {
    match error {
        Error::Type(message) => message,
        other => other.to_string(),
    }
}

/// One COPY row's values, read from a Python sequence as parameters are:
/// `None` is NULL, anything else a value whose kind is inferred.
fn row_values(row: &Bound<'_, PyAny>) -> PyResult<Vec<Option<ValueHolder>>> {
    if row.is_instance_of::<PyString>()
        || row.is_instance_of::<PyBytes>()
        || row.is_instance_of::<PyByteArray>()
    {
        return Err(PyTypeError::new_err(
            "a COPY row is a sequence of its columns' values, not text or bytes",
        ));
    }
    // A mapping or a set iterates, but over its keys or in no fixed order,
    // so only a sequence is a row.
    let items = row
        .cast::<PySequence>()
        .map_err(|_| PyTypeError::new_err("a COPY row is a sequence of its columns' values"))?
        .try_iter()?;
    items
        .map(|item| {
            let item = item?;
            if item.is_none() {
                Ok(None)
            } else {
                PyValue::coerce(&item).map(|value| Some(ValueHolder(value.inner)))
            }
        })
        .collect()
}

impl fmt::Debug for NativeCopyIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let finished = self.settle.lock().map(|settle| settle.is_none());
        f.debug_struct("NativeCopyIn")
            .field("finished", &finished.unwrap_or(true))
            .finish_non_exhaustive()
    }
}

#[pymethods]
impl NativeCopyIn {
    /// Queue one chunk, waiting while the queue is full; `False` once the
    /// COPY has stopped taking data, whose cause `finish` reports.
    fn write<'py>(&self, py: Python<'py>, data: &Bound<'_, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        self.connection.pool.check_owner(py)?;
        let data = chunk(data)?;
        let messages = lock(&self.messages)?
            .clone()
            .ok_or_else(|| LifecycleError::new_err("COPY has ended"))?;
        future_into_py(py, async move {
            Ok(messages.send(Message::Data(data)).await.is_ok())
        })
    }

    /// Encode `rows`, each a sequence of its columns' values, in the binary
    /// format — the header before the first — and queue them as one chunk,
    /// as `write` queues data. A row of another width, or a value its
    /// column's type does not take, raises a construction error naming the
    /// column, and nothing of the batch is queued.
    // [spec:pgorm:req:bindings.copy-rows+1]
    fn write_rows<'py>(
        &self,
        py: Python<'py>,
        rows: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.connection.pool.check_owner(py)?;
        let state = self
            .rows
            .as_ref()
            .ok_or_else(|| LifecycleError::new_err("write_rows writes the rows of copy_in_rows"))?;
        let encoder = CopyRowEncoder::new(described(&state.columns)?.to_vec());
        let mut out = BytesMut::new();
        for row in rows.try_iter()? {
            let values = row_values(&row?)?;
            let mut encoded = BytesMut::new();
            encoder
                .encode(&values, &mut encoded)
                .map_err(|error| ConstructionError::new_err(refusal(error)))?;
            out.extend_from_slice(&encoded);
        }
        let mut data = BytesMut::new();
        state.head(&mut data);
        data.extend_from_slice(&out);
        let messages = lock(&self.messages)?
            .clone()
            .ok_or_else(|| LifecycleError::new_err("COPY has ended"))?;
        let data = data.freeze();
        future_into_py(py, async move {
            Ok(messages.send(Message::Data(data)).await.is_ok())
        })
    }

    fn finish<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.connection.pool.check_owner(py)?;
        let messages = lock(&self.messages)?.take();
        let settle = lock(&self.settle)?
            .take()
            .ok_or_else(|| LifecycleError::new_err("COPY has ended"))?;
        let connection = self.connection.clone();
        let tail = self.rows.as_ref().map(|rows| {
            let mut tail = BytesMut::new();
            rows.head(&mut tail);
            CopyRowEncoder::trailer(&mut tail);
            tail.freeze()
        });
        future_into_py(py, async move {
            if let Some(messages) = messages {
                if let Some(tail) = tail {
                    let _ = messages.send(Message::Data(tail)).await;
                }
                let _ = messages.send(Message::Finish).await;
            }
            settle
                .wait(&connection)
                .await?
                .ok_or_else(|| InternalError::new_err("COPY was aborted before it finished"))
        })
    }

    /// Abandon the data: the server discards every row. The abandoned COPY's
    /// own outcome is not reported, so the exception that caused it is.
    fn abort<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.connection.pool.check_owner(py)?;
        drop(lock(&self.messages)?.take());
        let settle = lock(&self.settle)?.take();
        let connection = self.connection.clone();
        future_into_py(py, async move {
            if let Some(settle) = settle {
                let _ = settle.wait(&connection).await;
            }
            Ok(())
        })
    }

    fn closed(&self) -> PyResult<bool> {
        Ok(lock(&self.settle)?.is_none())
    }
}

/// What a COPY TO in progress holds.
enum Holder {
    /// Cancelled once the pump has restored or discarded the connection.
    Connection(CancellationToken),
    Transaction(Lease, Arc<State>),
}

struct Active {
    pieces: mpsc::Receiver<Piece>,
    stashed: Option<Piece>,
    holder: Holder,
}

impl Active {
    /// The data has ended and the owner is free for the next statement.
    fn complete(self) {
        if let Holder::Transaction(lease, _) = self.holder {
            lease.complete();
        }
    }

    /// Stop reading part way, which discards the connection — and with it a
    /// transaction — and wait until it is gone, so the close is observable.
    async fn abandon(self) {
        drop(self.pieces);
        match self.holder {
            Holder::Connection(stopped) => stopped.cancelled().await,
            Holder::Transaction(lease, state) => {
                drop(lease);
                state.released().await;
            }
        }
    }

    /// Join `first` with the chunks already received, up to `GATHERED`
    /// bytes. COPY promises no chunk boundary, so a pull is any slice of the
    /// byte stream; joining saves a Python round trip per row.
    fn gather(&mut self, first: Bytes) -> Bytes {
        let mut joined: Option<BytesMut> = None;
        let mut length = first.len();
        while length < GATHERED {
            match self.pieces.try_recv() {
                Ok(Piece::Chunk(next)) => {
                    length += next.len();
                    joined
                        .get_or_insert_with(|| BytesMut::from(&first[..]))
                        .extend_from_slice(&next);
                }
                Ok(end) => {
                    self.stashed = Some(end);
                    break;
                }
                Err(_) => break,
            }
        }
        joined.map_or(first, BytesMut::freeze)
    }
}

struct OutState {
    connection: Arc<ConnectionState>,
    transaction: Option<Arc<State>>,
    active: Arc<Mutex<Option<Active>>>,
    closing: CancellationToken,
    finished: CancellationToken,
    /// Set for `copy_out_rows`, whose rows are decoded as they arrive.
    rows: Option<RowsOut>,
}

/// A COPY of rows' reading side: the columns described before it started,
/// and the decoder over them, made at the first pull.
struct RowsOut {
    columns: Columns,
    decoder: StdMutex<Option<CopyRowDecoder>>,
}

impl RowsOut {
    /// The rows the data holds once `chunk` is added, each field read as a
    /// result column of its type is; with no chunk, the data's end is checked.
    fn rows(&self, chunk: Option<&[u8]>) -> PyResult<Vec<Vec<PyValue>>> {
        let mut slot = lock(&self.decoder)?;
        let decoder = match slot.as_mut() {
            Some(decoder) => decoder,
            None => slot.insert(CopyRowDecoder::new(described(&self.columns)?.to_vec())),
        };
        match chunk {
            Some(chunk) => decoder.push(chunk),
            None => decoder
                .finish()
                .map_err(|error| DecodeError::new_err(refusal(error)))?,
        }
        let mut rows = Vec::new();
        while let Some(fields) = decoder
            .next_fields()
            .map_err(|error| DecodeError::new_err(refusal(error)))?
        {
            let row = fields
                .iter()
                .zip(decoder.columns())
                .enumerate()
                .map(|(index, (raw, column))| {
                    value(Field::Copy {
                        column,
                        raw: raw.as_deref(),
                        index,
                    })
                })
                .collect::<PyResult<Vec<_>>>()?;
            rows.push(row);
        }
        Ok(rows)
    }
}

/// `rows` as a list of tuples of Python values.
fn rows_to_python(rows: Vec<Vec<PyValue>>) -> PyResult<Py<PyList>> {
    Python::attach(|py| {
        let rows = rows
            .iter()
            .map(|row| {
                let values = row
                    .iter()
                    .map(|value| value.to_python(py))
                    .collect::<PyResult<Vec<_>>>()?;
                PyTuple::new(py, values)
            })
            .collect::<PyResult<Vec<_>>>()?;
        PyList::new(py, rows).map(Bound::unbind)
    })
}

impl OutState {
    fn ensure_open(&self) -> PyResult<()> {
        match &self.transaction {
            Some(transaction) if transaction.closed() => {
                Err(LifecycleError::new_err("transaction is closed"))
            }
            _ => self.connection.ensure_open(),
        }
    }
}

// [spec:pgorm:req:bindings.copy]
// [spec:pgorm:req:bindings.cancellation]
/// The data side of a COPY TO. The end of the data frees the connection or
/// transaction; closing before it discards the connection.
#[pyclass(name = "_CopyOut", module = "pgorm._native", frozen)]
pub(crate) struct NativeCopyOut {
    state: Arc<OutState>,
}

impl fmt::Debug for NativeCopyOut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeCopyOut")
            .field("closed", &self.closed())
            .finish()
    }
}

#[pymethods]
impl NativeCopyOut {
    fn next<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.connection.pool.check_owner(py)?;
        let state = self.state.clone();
        future_into_py(py, async move {
            state.ensure_open()?;
            if state.finished.is_cancelled() {
                return Ok(None);
            }
            let mut slot = state.active.clone().try_lock_owned().map_err(|_| {
                LifecycleError::new_err("copy stream already has an active next operation")
            })?;
            let Some(active) = slot.as_mut() else {
                return Ok(None);
            };
            let piece = match active.stashed.take() {
                Some(piece) => Some(piece),
                None => tokio::select! {
                    _ = state.closing.cancelled() => return Err(LifecycleError::new_err("copy stream is closed")),
                    piece = active.pieces.recv() => piece,
                },
            };
            match piece {
                Some(Piece::Chunk(first)) => {
                    let data = active.gather(first);
                    Ok(Some(Python::attach(|py| PyBytes::new(py, &data).unbind())))
                }
                Some(Piece::End(end)) => {
                    let (result, operation) = *end;
                    if let Some(operation) = operation {
                        operation.restore();
                    }
                    if let Some(active) = slot.take() {
                        active.complete();
                    }
                    state.finished.cancel();
                    result
                        .map(|()| None)
                        .map_err(|error| database_error(error, &state.connection.pool.secrets))
                }
                None => {
                    drop(slot.take());
                    state.finished.cancel();
                    state.ensure_open()?;
                    Err(LifecycleError::new_err("COPY ended without completing"))
                }
            }
        })
    }

    /// The rows that have arrived, at least one, as a list of tuples of
    /// values; `None` at the end. A field that does not decode raises a
    /// decode error and closes the stream, discarding its connection.
    // [spec:pgorm:req:bindings.copy-rows+1]
    fn next_rows<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.connection.pool.check_owner(py)?;
        if self.state.rows.is_none() {
            return Err(LifecycleError::new_err(
                "next_rows reads the rows of copy_out_rows",
            ));
        }
        let state = self.state.clone();
        future_into_py(py, async move {
            state.ensure_open()?;
            if state.finished.is_cancelled() {
                return Ok(None);
            }
            let mut slot = state.active.clone().try_lock_owned().map_err(|_| {
                LifecycleError::new_err("copy stream already has an active next operation")
            })?;
            let rows_state = state
                .rows
                .as_ref()
                .ok_or_else(|| InternalError::new_err("COPY of rows lost its columns"))?;
            loop {
                let Some(active) = slot.as_mut() else {
                    return Ok(None);
                };
                let piece = match active.stashed.take() {
                    Some(piece) => Some(piece),
                    None => tokio::select! {
                        _ = state.closing.cancelled() => return Err(LifecycleError::new_err("copy stream is closed")),
                        piece = active.pieces.recv() => piece,
                    },
                };
                match piece {
                    Some(Piece::Chunk(first)) => {
                        let data = active.gather(first);
                        match rows_state.rows(Some(&data)) {
                            Ok(rows) if rows.is_empty() => {}
                            Ok(rows) => return rows_to_python(rows).map(Some),
                            Err(error) => {
                                if let Some(active) = slot.take() {
                                    active.abandon().await;
                                }
                                state.closing.cancel();
                                state.finished.cancel();
                                return Err(error);
                            }
                        }
                    }
                    Some(Piece::End(end)) => {
                        let (result, operation) = *end;
                        if let Some(operation) = operation {
                            operation.restore();
                        }
                        if let Some(active) = slot.take() {
                            active.complete();
                        }
                        state.finished.cancel();
                        result.map_err(|error| {
                            database_error(error, &state.connection.pool.secrets)
                        })?;
                        let rows = rows_state.rows(None)?;
                        return match rows.is_empty() {
                            true => Ok(None),
                            false => rows_to_python(rows).map(Some),
                        };
                    }
                    None => {
                        drop(slot.take());
                        state.finished.cancel();
                        state.ensure_open()?;
                        return Err(LifecycleError::new_err("COPY ended without completing"));
                    }
                }
            }
        })
    }

    /// The names of the columns each row holds, in order, as the server
    /// describes them.
    // [spec:pgorm:req:bindings.copy-rows+1]
    fn columns(&self) -> PyResult<Vec<String>> {
        let rows =
            self.state.rows.as_ref().ok_or_else(|| {
                LifecycleError::new_err("columns names the columns of copy_out_rows")
            })?;
        Ok(described(&rows.columns)?
            .iter()
            .map(|column| column.name().to_owned())
            .collect())
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.connection.pool.check_owner(py)?;
        self.state.closing.cancel();
        let state = self.state.clone();
        future_into_py(py, async move {
            let active = state.active.lock().await.take();
            state.finished.cancel();
            if let Some(active) = active {
                active.abandon().await;
            }
            Ok(())
        })
    }

    fn closed(&self) -> bool {
        self.state.closing.is_cancelled() || self.state.finished.is_cancelled()
    }
}

impl Drop for NativeCopyOut {
    fn drop(&mut self) {
        self.state.closing.cancel();
        if let Ok(mut slot) = self.state.active.try_lock() {
            drop(slot.take());
        }
    }
}
