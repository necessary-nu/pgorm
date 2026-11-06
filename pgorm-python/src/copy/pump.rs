//! The data flow between a native COPY handle and the task that owns the
//! connection or transaction the sink or stream borrows.

use std::sync::{Arc, OnceLock};

use futures_util::{SinkExt, StreamExt};
use pgorm::{Bytes, CopyColumn, CopyInSink, CopyOutStream, CopyTrait, Error};
use tokio::sync::mpsc;

use super::{CopyFrom, CopyTo};
use crate::runtime::Operation;

/// A COPY whose columns are described without it starting.
pub(crate) enum Described {
    From(CopyFrom),
    To(CopyTo),
}

/// The columns `statement` reads or writes, in order, as pgorm describes them
/// before a COPY starts, starting none.
// [spec:pgorm:req:bindings.copy-rows+2]
pub(crate) async fn describe<C>(db: &mut C, statement: &Described) -> Result<Vec<CopyColumn>, Error>
where
    C: CopyTrait + Send,
{
    match statement {
        Described::From(CopyFrom::Text(statement)) => db.copy_in_columns(statement).await,
        Described::From(CopyFrom::Csv(statement)) => db.copy_in_columns(statement).await,
        Described::From(CopyFrom::Binary(statement)) => db.copy_in_columns(statement).await,
        Described::To(CopyTo::Text(statement)) => db.copy_out_columns(statement).await,
        Described::To(CopyTo::Csv(statement)) => db.copy_out_columns(statement).await,
        Described::To(CopyTo::Binary(statement)) => db.copy_out_columns(statement).await,
    }
}

/// Chunks queued between Python and the owning task, in either direction.
/// The bound is what holds a fast source, or an unread server, back.
pub(crate) const QUEUED: usize = 4;

/// What a COPY FROM handle sends its owner. Every sender gone without a
/// `Finish` aborts the COPY.
#[derive(Debug)]
pub(crate) enum Message {
    Data(Bytes),
    Finish,
}

/// What a COPY TO owner sends its handle. A Connection's `End` carries its
/// lease, so the connection stays reserved until the reader reaches the end,
/// and is discarded if the reader closes first.
pub(crate) enum Piece {
    Chunk(Bytes),
    End(Box<(Result<(), Error>, Option<Operation>)>),
}

/// The columns a COPY of rows carries: described by the task owning the
/// connection before it starts the COPY, and read by the handle that encodes
/// or decodes the rows.
pub(crate) type Columns = Arc<OnceLock<Vec<CopyColumn>>>;

/// Why a COPY did not start.
#[derive(Debug)]
pub(crate) enum Unopened {
    /// The server refused the COPY at its start.
    Refused(Error),
    /// The server refused the description a COPY of rows starts with: a
    /// statement of its own, which leaves the connection as any failed
    /// statement does.
    Undescribed(Error),
}

impl Unopened {
    pub(crate) fn into_error(self) -> Error {
        match self {
            Self::Refused(error) | Self::Undescribed(error) => error,
        }
    }
}

// [spec:pgorm:req:bindings.copy-rows+2]
pub(crate) async fn open_in<'a, C>(
    db: &'a mut C,
    statement: &CopyFrom,
    rows: Option<&Columns>,
) -> Result<CopyInSink<'a>, Unopened>
where
    C: CopyTrait + Send,
{
    if let Some(rows) = rows {
        let columns = match statement {
            CopyFrom::Text(statement) => db.copy_in_columns(statement).await,
            CopyFrom::Csv(statement) => db.copy_in_columns(statement).await,
            CopyFrom::Binary(statement) => db.copy_in_columns(statement).await,
        };
        let _ = rows.set(columns.map_err(Unopened::Undescribed)?);
    }
    match statement {
        CopyFrom::Text(statement) => db.copy_in(statement).await,
        CopyFrom::Csv(statement) => db.copy_in(statement).await,
        CopyFrom::Binary(statement) => db.copy_in(statement).await,
    }
    .map_err(Unopened::Refused)
}

// [spec:pgorm:req:bindings.copy-rows+2]
pub(crate) async fn open_out<'a, C>(
    db: &'a mut C,
    statement: &CopyTo,
    rows: Option<&Columns>,
) -> Result<CopyOutStream<'a>, Unopened>
where
    C: CopyTrait + Send,
{
    if let Some(rows) = rows {
        let columns = match statement {
            CopyTo::Text(statement) => db.copy_out_columns(statement).await,
            CopyTo::Csv(statement) => db.copy_out_columns(statement).await,
            CopyTo::Binary(statement) => db.copy_out_columns(statement).await,
        };
        let _ = rows.set(columns.map_err(Unopened::Undescribed)?);
    }
    match statement {
        CopyTo::Text(statement) => db.copy_out(statement).await,
        CopyTo::Csv(statement) => db.copy_out(statement).await,
        CopyTo::Binary(statement) => db.copy_out(statement).await,
    }
    .map_err(Unopened::Refused)
}

/// Write each chunk into `sink` until `Finish` completes the COPY with its
/// row count. Every sender gone first answers `None`, having dropped the
/// sink, which aborts the COPY.
// [spec:pgorm:req:bindings.copy]
pub(crate) async fn drive_in(
    mut sink: CopyInSink<'_>,
    messages: &mut mpsc::Receiver<Message>,
) -> Result<Option<u64>, Error> {
    while let Some(message) = messages.recv().await {
        match message {
            Message::Data(chunk) => {
                sink.feed(chunk).await?;
                // Flushing only when nothing else is queued lets a burst of
                // chunks share the sink's buffering.
                if messages.is_empty() {
                    sink.flush().await?;
                }
            }
            Message::Finish => return sink.finish().await.map(Some),
        }
    }
    Ok(None)
}

/// Forward each chunk of `stream` until it ends, answering how; `None` once
/// nobody reads, with the stream dropped part way.
// [spec:pgorm:req:bindings.copy]
pub(crate) async fn drive_out(
    mut stream: CopyOutStream<'_>,
    pieces: &mpsc::Sender<Piece>,
) -> Option<Result<(), Error>> {
    loop {
        // Reserving before reading leaves an unread stream unpolled, so the
        // server is held back rather than buffered.
        let permit = pieces.reserve().await.ok()?;
        let next = tokio::select! {
            _ = pieces.closed() => return None,
            next = stream.next() => next,
        };
        match next {
            Some(Ok(chunk)) => permit.send(Piece::Chunk(chunk)),
            Some(Err(error)) => return Some(Err(error)),
            None => return Some(Ok(())),
        }
    }
}
