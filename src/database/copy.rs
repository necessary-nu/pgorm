use std::{
    fmt,
    future::poll_fn,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll, ready},
};

use bytes::Bytes;
use futures::{Sink, Stream};
use pgorm_pool::GenericClient;
use pgorm_query::{CopyBinary, CopyFormat, CopyFromStatement, CopyToStatement};

use super::copy_rows::{self, CopyColumn, CopyRowSink, CopyRowStream};
use crate::{DatabaseConnection, DatabaseTransaction, Error};

/// COPY's data flow: `COPY … FROM STDIN` and `COPY … TO STDOUT` on a
/// connection or a transaction.
///
/// A COPY's rows travel as `CopyData` messages beside the statement rather
/// than as a statement's parameters or result, so [`ConnectionTrait`] has no
/// route to them. These two methods are that route, over the typed statements
/// pgorm-query builds: the table and column names are quoted, the format is
/// one of PostgreSQL's three, and a statement that is not a COPY — which the
/// driver would run to completion before noticing it opened no data flow —
/// cannot be passed at all.
///
/// Both borrow the connection exclusively for as long as the sink or stream
/// they return is alive. A COPY in progress holds the connection's request
/// queue, so a second statement sent meanwhile would wait behind data the
/// caller had not yet sent or read; and a transaction committed while its COPY
/// is open would wait the same way. The borrow makes both a compile error:
///
/// ```compile_fail,E0499
/// # use pgorm::{CopyTrait, ConnectionTrait, DatabaseConnection, Error};
/// # use pgorm::pgorm_query::{CopyFromStatement, CopyText, Name};
/// async fn interleave(db: &mut DatabaseConnection) -> Result<(), Error> {
///     let copy = CopyFromStatement::new(Name::runtime("t"), CopyText);
///     let sink = db.copy_in(&copy).await?;
///     db.copy_in(&copy).await?;
///     sink.finish().await?;
///     Ok(())
/// }
/// ```
///
/// COPY takes no bind parameters, so the statement is sent as its inlined
/// rendering, and it is prepared afresh rather than through the statement
/// cache: the text of a COPY over a query carries that query's values, so
/// caching it would fill the bounded cache with statements run once.
///
/// [`ConnectionTrait`]: crate::ConnectionTrait
// [spec:pgorm:req:conn.copy+2]
#[async_trait::async_trait]
pub trait CopyTrait {
    /// Start `COPY … FROM STDIN`, returning the sink its data is written to.
    ///
    /// The server has accepted the statement when this returns; a table or
    /// column that does not exist, a column list naming a generated column,
    /// or an option the server refuses, is the error here. A row the server
    /// cannot take — a constraint violation, a field that does not parse — is
    /// reported by [`CopyInSink::finish`].
    ///
    /// A generated column, stored or virtual, takes no data, so list only the
    /// columns the rows supply: the server refuses a list naming one with
    /// SQLSTATE `42P10`, and pgorm does not check the list first, since that
    /// would cost a catalogue read before every COPY. A statement with no
    /// column list already leaves the generated columns out, and the server
    /// computes them as the rows go in.
    ///
    /// A refusal here also closes the connection, and with it any transaction
    /// open on it, which ends without committing. The driver has by then sent
    /// a message whose answer no request is waiting for, and it would hand
    /// that answer to the next request on the connection, every later one
    /// reading the answer meant for the one before; closing is the only
    /// recovery its API allows. Every later call on the connection fails as
    /// closed, and the pool discards it rather than lending it again. The same
    /// holds when this future is dropped before it completes.
    async fn copy_in<F>(
        &mut self,
        statement: &CopyFromStatement<F>,
    ) -> Result<CopyInSink<'_>, Error>
    where
        F: CopyFormat;

    /// Start `COPY … TO STDOUT`, returning the stream its data arrives on.
    ///
    /// A `SELECT` source whose tie limit or WINDOW clause PostgreSQL's
    /// grammar refuses is refused here before anything is sent, as
    /// [`Error::QueryBuilder`] ([`CopyToStatement::check_source`]); a refusal
    /// from the server leaves the connection as it was.
    async fn copy_out<F>(
        &mut self,
        statement: &CopyToStatement<F>,
    ) -> Result<CopyOutStream<'_>, Error>
    where
        F: CopyFormat;

    /// The columns each row of `statement` supplies, in order, with their
    /// types, as the server describes them: the statement's column list, or
    /// every column of its table the server does not generate.
    ///
    /// The description prepares a `SELECT` of those columns and runs
    /// nothing; a table or column that does not exist is refused here, and
    /// the connection is left as it was.
    // [spec:pgorm:req:conn.copy.rows+4]
    async fn copy_in_columns<F>(
        &mut self,
        statement: &CopyFromStatement<F>,
    ) -> Result<Vec<CopyColumn>, Error>
    where
        F: CopyFormat;

    /// The columns each row of `statement` holds, in order, with their
    /// types, as the server describes them: its table's columns the server
    /// does not generate, or its statement's result columns. A write source
    /// is prepared, never run.
    // [spec:pgorm:req:conn.copy.rows+4]
    async fn copy_out_columns<F>(
        &mut self,
        statement: &CopyToStatement<F>,
    ) -> Result<Vec<CopyColumn>, Error>
    where
        F: CopyFormat;

    /// Start a binary `COPY … FROM STDIN` whose rows are written as pgorm
    /// [`Value`](pgorm_query::Value)s, each checked against the columns
    /// [`copy_in_columns`](Self::copy_in_columns) describes.
    ///
    /// A refusal of the description leaves the connection as it was; a
    /// refusal of the COPY itself closes it, as [`copy_in`](Self::copy_in)'s
    /// does. A column list naming a generated column is the second kind: the
    /// description prepares a `SELECT`, which reads a generated column like
    /// any other, so it is the COPY's start that refuses the list and closes
    /// the connection. Leave generated columns out of the list, or give no
    /// list, which leaves them out already.
    // [spec:pgorm:req:conn.copy.rows+4]
    async fn copy_in_rows(
        &mut self,
        statement: &CopyFromStatement<CopyBinary>,
    ) -> Result<CopyRowSink<'_>, Error>
    where
        Self: Send,
    {
        let columns = self.copy_in_columns(statement).await?;
        let sink = self.copy_in(statement).await?;
        Ok(CopyRowSink::new(sink, columns))
    }

    /// Start a binary `COPY … TO STDOUT` whose rows arrive as pgorm
    /// [`Value`](pgorm_query::Value)s, each field decoded by the type
    /// [`copy_out_columns`](Self::copy_out_columns) describes for its column.
    // [spec:pgorm:req:conn.copy.rows+4]
    async fn copy_out_rows(
        &mut self,
        statement: &CopyToStatement<CopyBinary>,
    ) -> Result<CopyRowStream<'_>, Error>
    where
        Self: Send,
    {
        let columns = self.copy_out_columns(statement).await?;
        let stream = self.copy_out(statement).await?;
        Ok(CopyRowStream::new(stream, columns))
    }
}

/// The data of a `COPY … FROM STDIN`, written as [`Bytes`] through its
/// [`Sink`] implementation and completed by [`finish`](CopyInSink::finish).
///
/// The bytes are the format's own encoding of the rows, split anywhere: a
/// chunk need not end at a row boundary, and a chunk of any length is sent as
/// messages of at most a mebibyte. The sink buffers small writes up to a few
/// kilobytes before sending them and applies backpressure beyond that, so
/// writing a large source through it streams rather than accumulates.
///
/// Dropping the sink without finishing aborts the COPY: the client sends
/// `CopyFail`, the server discards every row, and the statement fails. Outside
/// a transaction nothing is written; inside one, the transaction is aborted,
/// as by any failed statement, and only a rollback ends it. `Sink::close`
/// finishes the COPY as `finish` does but discards the row count.
// [spec:pgorm:req:conn.copy+2]
pub struct CopyInSink<'a> {
    sink: Pin<Box<tokio_postgres::CopyInSink<Bytes>>>,
    /// What is left of a chunk longer than one message carries, sent before
    /// the sink is ready for another.
    rest: Bytes,
    connection: PhantomData<&'a mut ()>,
}

/// The most one `CopyData` message carries. The driver sends every chunk
/// longer than four kilobytes as a single message, and PostgreSQL refuses a
/// message past 1 GiB by ending the session, so a longer chunk is split here;
/// a mebibyte also bounds the buffer one message costs the server.
const MESSAGE_BYTES: usize = 1 << 20;

impl CopyInSink<'_> {
    fn new(sink: tokio_postgres::CopyInSink<Bytes>) -> Self {
        Self {
            sink: Box::pin(sink),
            rest: Bytes::new(),
            connection: PhantomData,
        }
    }

    /// Send what remains of a split chunk, one message per readiness.
    fn poll_rest(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        while !self.rest.is_empty() {
            ready!(self.sink.as_mut().poll_ready(cx))?;
            let message = self.rest.split_to(self.rest.len().min(MESSAGE_BYTES));
            self.sink.as_mut().start_send(message)?;
        }
        Poll::Ready(Ok(()))
    }

    /// Complete the COPY, returning the number of rows it inserted.
    ///
    /// This is where a row the server refused is reported, as the
    /// [`Error::Postgres`] carrying its SQLSTATE. The protocol carries no
    /// error back while the client is still sending, so a refusal at the
    /// first row still arrives here, after the rest of the data has been sent
    /// and discarded. Cancelling this future once
    /// the data has been sent leaves the outcome unknown: the server may
    /// already have committed the rows, outside a transaction, so the
    /// connection's state is not something a caller can assume.
    pub async fn finish(mut self) -> Result<u64, Error> {
        poll_fn(|cx| self.poll_rest(cx)).await?;
        Ok(self.sink.as_mut().finish().await?)
    }
}

impl Sink<Bytes> for CopyInSink<'_> {
    type Error = Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        ready!(self.poll_rest(cx))?;
        self.sink.as_mut().poll_ready(cx).map_err(Error::from)
    }

    fn start_send(mut self: Pin<&mut Self>, mut chunk: Bytes) -> Result<(), Error> {
        if chunk.len() > MESSAGE_BYTES {
            self.rest = chunk.split_off(MESSAGE_BYTES);
        }
        self.sink.as_mut().start_send(chunk).map_err(Error::from)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        ready!(self.poll_rest(cx))?;
        self.sink.as_mut().poll_flush(cx).map_err(Error::from)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        ready!(self.poll_rest(cx))?;
        self.sink.as_mut().poll_close(cx).map_err(Error::from)
    }
}

impl fmt::Debug for CopyInSink<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CopyInSink").finish_non_exhaustive()
    }
}

/// The data of a `COPY … TO STDOUT`, as a [`Stream`] of [`Bytes`] chunks in
/// the statement's format.
///
/// A chunk is one `CopyData` message: PostgreSQL sends one per row in the
/// text and CSV formats, but nothing promises that boundary, and in binary a
/// reader should treat the chunks as one byte stream. The stream ends after
/// the last chunk, and a failure part-way through is an `Err` item.
///
/// Rows are pulled as the stream is polled, so an unread stream holds the
/// server back rather than filling memory. Dropping it early does not stop
/// the server: the connection reads and discards the rest of the data before
/// the next statement on it runs.
// [spec:pgorm:req:conn.copy+2]
pub struct CopyOutStream<'a> {
    stream: Pin<Box<tokio_postgres::CopyOutStream>>,
    connection: PhantomData<&'a mut ()>,
}

impl CopyOutStream<'_> {
    fn new(stream: tokio_postgres::CopyOutStream) -> Self {
        Self {
            stream: Box::pin(stream),
            connection: PhantomData,
        }
    }
}

impl Stream for CopyOutStream<'_> {
    type Item = Result<Bytes, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.stream
            .as_mut()
            .poll_next(cx)
            .map(|chunk| chunk.map(|chunk| chunk.map_err(Error::from)))
    }
}

impl fmt::Debug for CopyOutStream<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CopyOutStream").finish_non_exhaustive()
    }
}

// [spec:pgorm:req:conn.copy+2]    COPY on the pooled client
#[async_trait::async_trait]
impl CopyTrait for DatabaseConnection {
    async fn copy_in<F>(
        &mut self,
        statement: &CopyFromStatement<F>,
    ) -> Result<CopyInSink<'_>, Error>
    where
        F: CopyFormat,
    {
        let prepared = self.0.prepare(&statement.to_string()).await?;
        let sink = self.0.copy_in::<_, Bytes>(&prepared).await?;
        Ok(CopyInSink::new(sink))
    }

    async fn copy_out<F>(
        &mut self,
        statement: &CopyToStatement<F>,
    ) -> Result<CopyOutStream<'_>, Error>
    where
        F: CopyFormat,
    {
        statement.check_source()?;
        let sql = statement.to_string();
        let stream = self.0.copy_out::<str>(sql.as_str()).await?;
        Ok(CopyOutStream::new(stream))
    }

    async fn copy_in_columns<F>(
        &mut self,
        statement: &CopyFromStatement<F>,
    ) -> Result<Vec<CopyColumn>, Error>
    where
        F: CopyFormat,
    {
        copy_rows::copy_in_columns(&self.0, statement).await
    }

    async fn copy_out_columns<F>(
        &mut self,
        statement: &CopyToStatement<F>,
    ) -> Result<Vec<CopyColumn>, Error>
    where
        F: CopyFormat,
    {
        copy_rows::copy_out_columns(&self.0, statement).await
    }
}

// [spec:pgorm:req:conn.copy+2]    COPY inside a transaction or savepoint
#[async_trait::async_trait]
impl CopyTrait for DatabaseTransaction<'_> {
    async fn copy_in<F>(
        &mut self,
        statement: &CopyFromStatement<F>,
    ) -> Result<CopyInSink<'_>, Error>
    where
        F: CopyFormat,
    {
        let prepared = GenericClient::prepare(self.tx(), &statement.to_string()).await?;
        let sink = self.tx().copy_in::<_, Bytes>(&prepared).await?;
        Ok(CopyInSink::new(sink))
    }

    async fn copy_out<F>(
        &mut self,
        statement: &CopyToStatement<F>,
    ) -> Result<CopyOutStream<'_>, Error>
    where
        F: CopyFormat,
    {
        statement.check_source()?;
        let sql = statement.to_string();
        let stream = self.tx().copy_out::<str>(sql.as_str()).await?;
        Ok(CopyOutStream::new(stream))
    }

    async fn copy_in_columns<F>(
        &mut self,
        statement: &CopyFromStatement<F>,
    ) -> Result<Vec<CopyColumn>, Error>
    where
        F: CopyFormat,
    {
        copy_rows::copy_in_columns(self.tx(), statement).await
    }

    async fn copy_out_columns<F>(
        &mut self,
        statement: &CopyToStatement<F>,
    ) -> Result<Vec<CopyColumn>, Error>
    where
        F: CopyFormat,
    {
        copy_rows::copy_out_columns(self.tx(), statement).await
    }
}
