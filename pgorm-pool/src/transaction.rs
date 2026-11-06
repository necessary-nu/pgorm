//! The transaction wrappers: a [`tokio_postgres::Transaction`] and its
//! builder, each carrying the [`StatementCache`] of the client that opened it.

use std::{
    fmt,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use bytes::Buf;
use futures_util::StreamExt;
use tokio::task::AbortHandle;
use tokio_postgres::{
    CopyInSink, CopyOutStream, Error, IsolationLevel, Row, RowStream, SimpleQueryMessage,
    Statement, ToStatement, Transaction as PgTransaction,
    TransactionBuilder as PgTransactionBuilder,
    types::{BorrowToSql, ToSql, Type},
};

use crate::StatementCache;

/// Wrapper around [`tokio_postgres::Transaction`] with a [`StatementCache`]
/// from the [`Client`](crate::Client) object it was created by.
///
/// It forwards the statement methods rather than dereferencing to the inner
/// transaction. `tokio_postgres::Transaction::savepoint` writes its name
/// into `SAVEPOINT {name}` unquoted, so a `Deref` to it made
/// [`savepoint`](Self::savepoint)'s quoting optional; without one, the
/// quoted method is the only route to a savepoint name:
///
/// ```compile_fail,E0308
/// async fn bypass(txn: &mut pgorm_pool::Transaction<'_>) {
///     let _ = pgorm_pool::tokio_postgres::Transaction::savepoint(&mut *txn, "x").await;
/// }
/// ```
// [spec:pgorm:req:conn.pool.savepoint-name+3]
pub struct Transaction<'a> {
    /// Original [`PgTransaction`].
    pub(crate) txn: PgTransaction<'a>,

    /// [`StatementCache`] of this [`Transaction`].
    pub statement_cache: Arc<StatementCache>,

    /// The task driving the connection, which [`copy_in`](Self::copy_in)
    /// aborts when a COPY's refusal leaves the connection unusable.
    pub(crate) connection: AbortHandle,

    /// Whether this is a savepoint inside another transaction, which
    /// [`commit`](Self::commit) ends with `RELEASE`, not `COMMIT`.
    pub(crate) savepoint: bool,
}

impl Transaction<'_> {
    /// Like [`tokio_postgres::Transaction::prepare()`], but uses an existing
    /// [`Statement`] from the [`StatementCache`] if possible.
    pub async fn prepare_cached(&self, query: &str) -> Result<Statement, Error> {
        self.statement_cache.prepare(self.txn.client(), query).await
    }

    /// Like [`tokio_postgres::Transaction::prepare_typed()`], but uses an
    /// existing [`Statement`] from the [`StatementCache`] if possible.
    pub async fn prepare_typed_cached(
        &self,
        query: &str,
        types: &[Type],
    ) -> Result<Statement, Error> {
        self.statement_cache
            .prepare_typed(self.txn.client(), query, types)
            .await
    }

    /// Like [`tokio_postgres::Transaction::execute()`].
    pub async fn execute<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, Error>
    where
        T: ?Sized + ToStatement,
    {
        self.txn.execute(statement, params).await
    }

    /// Like [`tokio_postgres::Transaction::execute_raw()`].
    pub async fn execute_raw<P, I, T>(&self, statement: &T, params: I) -> Result<u64, Error>
    where
        T: ?Sized + ToStatement,
        P: BorrowToSql,
        I: IntoIterator<Item = P>,
        I::IntoIter: ExactSizeIterator,
    {
        self.txn.execute_raw(statement, params).await
    }

    /// Like [`tokio_postgres::Transaction::query()`].
    pub async fn query<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, Error>
    where
        T: ?Sized + ToStatement,
    {
        self.txn.query(statement, params).await
    }

    /// Like [`tokio_postgres::Transaction::query_one()`].
    pub async fn query_one<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Row, Error>
    where
        T: ?Sized + ToStatement,
    {
        self.txn.query_one(statement, params).await
    }

    /// Like [`tokio_postgres::Transaction::query_opt()`].
    pub async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, Error>
    where
        T: ?Sized + ToStatement,
    {
        self.txn.query_opt(statement, params).await
    }

    /// Like [`tokio_postgres::Transaction::query_raw()`].
    pub async fn query_raw<T, P, I>(&self, statement: &T, params: I) -> Result<RowStream, Error>
    where
        T: ?Sized + ToStatement,
        P: BorrowToSql,
        I: IntoIterator<Item = P>,
        I::IntoIter: ExactSizeIterator,
    {
        self.txn.query_raw(statement, params).await
    }

    /// Like [`tokio_postgres::Transaction::batch_execute()`].
    ///
    /// Statement text runs as written through the simple query protocol, as
    /// on the client; it is the caller's SQL, not a name this wrapper
    /// composes.
    pub async fn batch_execute(&self, query: &str) -> Result<(), Error> {
        self.txn.batch_execute(query).await
    }

    /// Like [`tokio_postgres::Transaction::copy_in()`], but a COPY the server
    /// refuses before its data flow starts closes the connection, as
    /// [`ClientWrapper::copy_in`](crate::ClientWrapper::copy_in) does and for
    /// the same reason. The transaction then ends without committing.
    // [spec:pgorm:req:conn.copy+2]    the transaction wrapper forwards COPY FROM STDIN
    pub async fn copy_in<T, U>(&self, statement: &T) -> Result<CopyInSink<U>, Error>
    where
        T: ?Sized + ToStatement,
        U: Buf + 'static + Send,
    {
        crate::copy::guarded(&self.connection, self.txn.copy_in(statement)).await
    }

    /// Like [`tokio_postgres::Transaction::copy_out()`].
    // [spec:pgorm:req:conn.copy+2]    the transaction wrapper forwards COPY TO STDOUT
    pub async fn copy_out<T>(&self, statement: &T) -> Result<CopyOutStream, Error>
    where
        T: ?Sized + ToStatement,
    {
        self.txn.copy_out(statement).await
    }

    /// Like [`tokio_postgres::Transaction::commit()`], but a transaction a
    /// failed statement has aborted is rolled back and reported as
    /// [`CommitError::Aborted`].
    ///
    /// PostgreSQL answers the `COMMIT` of an aborted transaction with a
    /// `ROLLBACK` command tag and no error, and tokio-postgres reads no tag.
    /// So the `COMMIT` is sent behind a statement that asks first: an empty
    /// `SELECT`, which an aborted transaction refuses with SQLSTATE `25P02`
    /// (`in_failed_sql_transaction`), and whose refusal skips the `COMMIT`
    /// sent with it in the same simple query. A savepoint is asked the same
    /// way, a round trip ahead of its `RELEASE`: an aborted savepoint refuses
    /// the `RELEASE` too, but the `RELEASE` consumes the handle the
    /// `ROLLBACK TO` would be sent through.
    // [spec:pgorm:sem:conn.tx.aborted]
    pub async fn commit(self) -> Result<(), CommitError> {
        if self.savepoint {
            if let Err(refusal) = self.txn.batch_execute("SELECT").await {
                return Err(self.aborted(refusal).await);
            }
            return self.txn.commit().await.map_err(CommitError::Failed);
        }
        match committed(&self.txn).await {
            Ok(()) => {
                self.ended();
                Ok(())
            }
            Err(Ending::Asking(refusal)) => Err(self.aborted(refusal).await),
            Err(Ending::Committing(error)) => {
                self.ended();
                Err(CommitError::Failed(error))
            }
        }
    }

    /// Roll back a transaction the server refused the asking statement in:
    /// aborted, so nothing in it can commit. A refusal that is not the
    /// server's is the connection breaking, which leaves nothing to send a
    /// `ROLLBACK` through.
    async fn aborted(self, refusal: Error) -> CommitError {
        if refusal.as_db_error().is_none() {
            return CommitError::Failed(refusal);
        }
        CommitError::Aborted {
            rollback: self.txn.rollback().await.err(),
            refusal,
        }
    }

    /// Give the transaction up once the server has ended it. Dropping the
    /// inner transaction would queue a `ROLLBACK` behind the `COMMIT` that
    /// ended it, which the server would answer with a warning that no
    /// transaction is in progress. A transaction opened on a client holds
    /// nothing but its borrow of the client, so forgetting it frees nothing
    /// and leaves the borrow to end as this handle does.
    fn ended(self) {
        let Self { txn, .. } = self;
        std::mem::forget(txn);
    }

    /// Like [`tokio_postgres::Transaction::rollback()`].
    pub async fn rollback(self) -> Result<(), Error> {
        self.txn.rollback().await
    }

    /// Like [`tokio_postgres::Transaction::transaction()`], but returns a
    /// wrapped [`Transaction`] with a [`StatementCache`].
    #[allow(unused_lifetimes)] // false positive
    pub async fn transaction(&mut self) -> Result<Transaction<'_>, Error> {
        Ok(Transaction {
            txn: PgTransaction::transaction(&mut self.txn).await?,
            statement_cache: self.statement_cache.clone(),
            connection: self.connection.clone(),
            savepoint: true,
        })
    }

    /// Like [`tokio_postgres::Transaction::savepoint()`], but returns a wrapped
    /// [`Transaction`] with a [`StatementCache`], and quotes `name` as an
    /// identifier.
    ///
    /// `SAVEPOINT` takes no parameters, so the name is the one part of the
    /// statement that can only travel as SQL text. `tokio_postgres` writes it
    /// into `SAVEPOINT {name}` — and, from the handle returned here, into
    /// `RELEASE {name}` and `ROLLBACK TO {name}` — through the simple query
    /// protocol, which accepts several statements at once. A name is therefore
    /// quoted before it is handed over: any `"` in it is doubled and the whole
    /// is wrapped in `"`, which is total over every name PostgreSQL can carry,
    /// so `x"; DROP TABLE t; --` names a savepoint and nothing else.
    ///
    /// Two consequences worth knowing. The name keeps its case and its
    /// punctuation rather than being folded to lower case, because that is
    /// what quoting an identifier means; within this handle that is invisible,
    /// since the release and the rollback quote it the same way. And a name
    /// holding a NUL byte is refused — PostgreSQL carries no identifier
    /// containing one under any quoting at all, and the protocol encoder
    /// rejects it before a byte reaches the server, so the `Err` arrives with
    /// the transaction untouched.
    // [spec:pgorm:req:conn.pool.savepoint-name+3]
    #[allow(unused_lifetimes)] // false positive
    pub async fn savepoint<I>(&mut self, name: I) -> Result<Transaction<'_>, Error>
    where
        I: Into<String>,
    {
        Ok(Transaction {
            txn: PgTransaction::savepoint(&mut self.txn, quote_identifier(&name.into())).await?,
            statement_cache: self.statement_cache.clone(),
            connection: self.connection.clone(),
            savepoint: true,
        })
    }
}

/// `name` as a PostgreSQL quoted identifier: every `"` doubled, the whole
/// wrapped in `"`.
///
/// Total by construction. The delimiter is the only character a quoted
/// identifier gives special meaning to, and doubling is how PostgreSQL spells
/// an escaped one, so there is no name this can fail on and no name it can
/// turn into something other than an identifier.
// [spec:pgorm:req:conn.pool.savepoint-name+3]
fn quote_identifier(name: &str) -> String {
    let mut quoted = String::with_capacity(name.len() + 2);
    quoted.push('"');
    for character in name.chars() {
        if character == '"' {
            quoted.push('"');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

/// Where a transaction's `SELECT; COMMIT` failed: in the `SELECT` that asks
/// whether the transaction is aborted, the `COMMIT` then skipped, or in the
/// `COMMIT`, which ends the transaction whether or not it fails.
enum Ending {
    Asking(Error),
    Committing(Error),
}

/// Send `SELECT; COMMIT` as one simple query, and say which statement failed
/// by whether the `SELECT` had completed.
// [spec:pgorm:sem:conn.tx.aborted]
async fn committed(txn: &PgTransaction<'_>) -> Result<(), Ending> {
    let answers = txn
        .client()
        .simple_query_raw("SELECT; COMMIT")
        .await
        .map_err(Ending::Asking)?;
    let mut answers = std::pin::pin!(answers);
    let mut asked = false;
    while let Some(answer) = answers.next().await {
        match answer {
            Ok(SimpleQueryMessage::CommandComplete(_)) => asked = true,
            Ok(_) => {}
            Err(error) if asked => return Err(Ending::Committing(error)),
            Err(error) => return Err(Ending::Asking(error)),
        }
    }
    Ok(())
}

/// Why [`Transaction::commit`] committed nothing.
///
/// Exhaustive, so that pgorm, which turns each kind into an error of its
/// own, is told by the compiler when a kind is added.
// [spec:pgorm:sem:conn.tx.aborted]
#[derive(Debug)]
pub enum CommitError {
    /// The `COMMIT` or `RELEASE` failed, or the connection broke before the
    /// server answered. PostgreSQL ends a transaction whose `COMMIT` fails,
    /// so nothing is left to roll back.
    Failed(Error),
    /// A statement that failed inside the transaction had aborted it, so it
    /// could only roll back, and was rolled back (a savepoint rolled back
    /// to): nothing in it committed.
    Aborted {
        /// The server's refusal of the statement that asked, SQLSTATE
        /// `25P02` (`in_failed_sql_transaction`).
        refusal: Error,
        /// What the `ROLLBACK` or `ROLLBACK TO` that followed failed with,
        /// if it failed.
        rollback: Option<Error>,
    },
}

impl fmt::Display for CommitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(error) => write!(f, "{error}"),
            Self::Aborted { rollback: None, .. } => f.write_str(
                "a statement that failed inside the transaction had aborted it, so it was rolled back",
            ),
            Self::Aborted {
                rollback: Some(rollback),
                ..
            } => write!(
                f,
                "a statement that failed inside the transaction had aborted it, and its ROLLBACK failed: {rollback}"
            ),
        }
    }
}

impl std::error::Error for CommitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Failed(error) | Self::Aborted { refusal: error, .. } => Some(error),
        }
    }
}

impl fmt::Debug for Transaction<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transaction")
            //.field("txn", &self.txn)
            .field("statement_cache", &self.statement_cache)
            .finish()
    }
}

/// Wrapper around [`tokio_postgres::TransactionBuilder`] with a
/// [`StatementCache`] from the [`Client`](crate::Client) object it was created by.
#[must_use = "builder does nothing itself, use `.start()` to use it"]
pub struct TransactionBuilder<'a> {
    /// Original [`PgTransactionBuilder`].
    pub(crate) builder: PgTransactionBuilder<'a>,

    /// [`StatementCache`] of this [`TransactionBuilder`].
    pub(crate) statement_cache: Arc<StatementCache>,

    /// The task driving the connection, handed to the [`Transaction`].
    pub(crate) connection: AbortHandle,
}

impl<'a> TransactionBuilder<'a> {
    /// Sets the isolation level of the transaction.
    ///
    /// Like [`tokio_postgres::TransactionBuilder::isolation_level()`].
    pub fn isolation_level(self, isolation_level: IsolationLevel) -> Self {
        Self {
            builder: self.builder.isolation_level(isolation_level),
            ..self
        }
    }

    /// Sets the access mode of the transaction.
    ///
    /// Like [`tokio_postgres::TransactionBuilder::read_only()`].
    pub fn read_only(self, read_only: bool) -> Self {
        Self {
            builder: self.builder.read_only(read_only),
            ..self
        }
    }

    /// Sets the deferrability of the transaction.
    ///
    /// If the transaction is also serializable and read only, creation
    /// of the transaction may block, but when it completes the transaction
    /// is able to run with less overhead and a guarantee that it will not
    /// be aborted due to serialization failure.
    ///
    /// Like [`tokio_postgres::TransactionBuilder::deferrable()`].
    pub fn deferrable(self, deferrable: bool) -> Self {
        Self {
            builder: self.builder.deferrable(deferrable),
            ..self
        }
    }

    /// Begins the [`Transaction`].
    ///
    /// The transaction will roll back by default - use the commit method
    /// to commit it.
    ///
    /// Like [`tokio_postgres::TransactionBuilder::start()`].
    pub async fn start(self) -> Result<Transaction<'a>, Error> {
        Ok(Transaction {
            txn: self.builder.start().await?,
            statement_cache: self.statement_cache,
            connection: self.connection,
            savepoint: false,
        })
    }
}

impl fmt::Debug for TransactionBuilder<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransactionBuilder")
            //.field("builder", &self.builder)
            .field("statement_cache", &self.statement_cache)
            .finish()
    }
}

impl<'a> Deref for TransactionBuilder<'a> {
    type Target = PgTransactionBuilder<'a>;

    fn deref(&self) -> &Self::Target {
        &self.builder
    }
}

impl DerefMut for TransactionBuilder<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.builder
    }
}
