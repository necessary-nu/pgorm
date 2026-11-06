//! Transactions, which are opened only by running a closure inside one: what
//! the closure returns decides whether the transaction commits, so no handle
//! outlives that decision and none can be left open by forgetting it.

use std::future::Future;

use pgorm_pool::CommitError;

use crate::{DatabaseConnection, DatabaseTransaction, Error};

/// The means the closure form is built on — opening a transaction on a parent
/// and ending it — kept out of every other crate's reach.
///
/// The traits are public only so that they can bound [`TransactionTrait`] and
/// its handle. Their module is private, so no other crate can name them to
/// implement them, and every method takes a [`Token`] nothing outside pgorm
/// can construct, so none can call them either — not even through a
/// `C: TransactionTrait` bound, which puts a bound's methods in reach without
/// its trait being named.
///
/// Opening lives on the parent and is generic over the borrow alone, never in
/// an impl that ties the handle's lifetime to the borrow's: a spawned task's
/// `Send` check erases every lifetime in the future it holds and needs each
/// impl it meets to hold for any of them, and an impl relating two of them
/// holds only where they agree.
pub(crate) mod sealed {
    use std::future::Future;

    use super::TransactionTrait;
    use crate::{ConnectionTrait, Error};

    /// What every method of the sealed traits demands, so that only pgorm
    /// can call them.
    #[derive(Debug, Clone, Copy)]
    pub struct Token(());

    pub(crate) const TOKEN: Token = Token(());

    /// The handle a transaction closure is given.
    ///
    /// Declared apart from [`Open`]: a trait that both declares a
    /// lifetime-generic type and returns it from `&mut self` must bound it by
    /// `Self: 't`, and under a closure's signature, which is generic over
    /// every `'t`, that bound fails for any parent that is itself borrowed —
    /// a transaction opening a savepoint.
    pub trait Handle {
        /// [`DatabaseTransaction`](crate::DatabaseTransaction) on a connection
        /// or a transaction, and
        /// [`InstrumentedTransaction`](crate::metric::InstrumentedTransaction)
        /// on the metrics wrappers.
        type Transaction<'t>: ConnectionTrait + TransactionTrait + Close;
    }

    /// A parent a transaction opens on: `BEGIN` on a connection, a savepoint
    /// on a transaction.
    pub trait Open: Handle {
        /// Open the transaction, borrowing `self` exclusively for as long as
        /// the handle lives.
        fn open(
            &mut self,
            token: Token,
        ) -> impl Future<Output = Result<Self::Transaction<'_>, Error>> + Send;
    }

    /// How a transaction handle ends. Both consume it and await the server's
    /// answer, which for a savepoint is `RELEASE` or `ROLLBACK TO`.
    pub trait Close: Sized {
        fn commit(self, token: Token) -> impl Future<Output = Result<(), Error>> + Send;

        fn rollback(self, token: Token) -> impl Future<Output = Result<(), Error>> + Send;
    }
}

/// Run a closure inside a transaction: `BEGIN` on a connection, a savepoint
/// inside a transaction.
///
/// It is the only way to open one. The transaction lasts exactly as long as
/// the closure's future: `Ok` commits, `Err` rolls back, and a future dropped
/// before it finishes rolls back as its handle drops. The trait is sealed.
///
/// The closure is given `&mut Self::Transaction<'_>`, a handle borrowing the
/// receiver exclusively: a [`DatabaseTransaction`] on a connection or a
/// transaction, and an
/// [`InstrumentedTransaction`](crate::metric::InstrumentedTransaction) on the
/// metrics wrappers, so that the statements and savepoints inside stay
/// instrumented. Either is a [`ConnectionTrait`](crate::ConnectionTrait) and
/// itself a `TransactionTrait`, whose own transactions are savepoints.
///
/// The future [`transaction`](TransactionTrait::transaction) returns promises
/// no `Send`. A call on a concrete receiver — a [`DatabaseConnection`], a
/// [`DatabaseTransaction`], a metrics wrapper — is `Send` whenever the closure's
/// future is, so it runs in `tokio::spawn`. Code generic over
/// `C: TransactionTrait` gets a future the compiler does not know to be
/// `Send`: promising it would mean bounding an async closure's future, which
/// stable Rust cannot express.
// [spec:pgorm:req:conn.tx+3]
pub trait TransactionTrait: sealed::Open {
    /// Run `f` inside a transaction, committing when it returns `Ok` and
    /// rolling back when it returns `Err`.
    ///
    /// The rollback is awaited, so the transaction is over by the time this
    /// returns. `Ok` is returned only once `COMMIT` succeeded and committed;
    /// a failing `BEGIN` or `COMMIT` is [`TransactionError::Connection`], a
    /// transaction a statement failing inside it had aborted among them,
    /// though the closure caught that statement's error
    /// ([`Error::TransactionAborted`]), the
    /// closure's own error is [`TransactionError::Transaction`], and the
    /// closure's error followed by a failed `ROLLBACK` is
    /// [`TransactionError::RollbackFailed`]. Dropping the
    /// returned future before it finishes rolls the transaction back without
    /// awaiting the server.
    // [spec:pgorm:sem:conn.tx.closure+6]    one body for every implementor
    #[doc(alias = "begin")]
    fn transaction<T, E, F>(&mut self, f: F) -> impl Future<Output = Result<T, TransactionError<E>>>
    where
        F: AsyncFnOnce(&mut Self::Transaction<'_>) -> Result<T, E>,
    {
        async move {
            let txn = sealed::Open::open(self, sealed::TOKEN)
                .await
                .map_err(TransactionError::Connection)?;

            drive(txn, f).await
        }
    }
}

// [spec:pgorm:sem:conn.tx.closure+6]    commit on Ok, awaited rollback on Err
pub(crate) async fn drive<H, T, E, F>(mut txn: H, f: F) -> Result<T, TransactionError<E>>
where
    H: sealed::Close,
    F: AsyncFnOnce(&mut H) -> Result<T, E>,
{
    match f(&mut txn).await {
        Ok(value) => txn
            .commit(sealed::TOKEN)
            .await
            .map(|()| value)
            .map_err(TransactionError::Connection),
        Err(cause) => Err(rolled_back(txn, cause).await),
    }
}

/// Roll back a transaction whose closure failed with `cause`, and say how it
/// ended.
// [spec:pgorm:sem:conn.tx.closure+6]    a failed ROLLBACK travels beside the closure's error
pub(crate) async fn rolled_back<E>(txn: impl sealed::Close, cause: E) -> TransactionError<E> {
    match txn.rollback(sealed::TOKEN).await {
        Ok(()) => TransactionError::Transaction(cause),
        Err(rollback) => TransactionError::RollbackFailed { cause, rollback },
    }
}

/// Why a closure transaction did not commit.
///
/// The variants keep the closure's own failure distinct from a failure of the
/// transaction machinery itself, which is why there is deliberately no
/// `From<Error>` impl: with `E = Error` it would silently file every
/// closure-side error under `Connection` and erase that distinction. The
/// other direction exists: a `TransactionError<E>` whose `E` is an [`Error`],
/// or converts into one, converts into an `Error`, so `?` passes a savepoint's
/// failure to the closure around it.
// [spec:pgorm:sem:conn.tx.closure+6]    error wrapper
#[derive(Debug)]
#[non_exhaustive]
pub enum TransactionError<E> {
    /// `BEGIN`, `COMMIT`, or acquiring the transaction failed.
    ///
    /// [`Error::TransactionAborted`] is the `COMMIT` of a transaction that a
    /// failed statement had aborted, its error caught by the closure: rolled
    /// back, nothing in it committed, its connection in a known state.
    Connection(Error),
    /// The closure returned an error; the transaction was rolled back.
    Transaction(E),
    /// The closure returned `cause`, and the `ROLLBACK` that followed failed
    /// with `rollback`.
    ///
    /// How the transaction ended is then unknown, and so is the state of the
    /// connection it ran on: [`discard`](crate::DatabaseConnection::discard)
    /// it rather than let the pool lend it again.
    RollbackFailed {
        /// The closure's own error, which is why the transaction failed.
        cause: E,
        /// What the `ROLLBACK` failed with.
        rollback: Error,
    },
}

impl<E> std::fmt::Display for TransactionError<E>
where
    E: std::fmt::Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connection(error) => write!(f, "Transaction Connection Error: {error}"),
            Self::Transaction(error) => write!(f, "Transaction Error: {error}"),
            Self::RollbackFailed { cause, rollback } => {
                write!(
                    f,
                    "Transaction Error: {cause}; its ROLLBACK failed: {rollback}"
                )
            }
        }
    }
}

impl<E> std::error::Error for TransactionError<E>
where
    E: std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Connection(error) => Some(error),
            Self::Transaction(error) | Self::RollbackFailed { cause: error, .. } => Some(error),
        }
    }
}

/// So that `?` passes a savepoint's failure to the closure it is nested in,
/// as it passes a statement's: the savepoint's `transaction` returns a
/// `TransactionError<E>`, the enclosing closure an `Error`.
///
/// `Connection` and `Transaction` become the error they hold. A
/// `RollbackFailed` becomes [`Error::RollbackFailed`], holding both errors,
/// because the failed rollback is the one thing the enclosing transaction
/// has to be told: keeping only the cause would report a savepoint that
/// rolled back cleanly.
// [spec:pgorm:sem:conn.tx.compose]
impl<E: Into<Error>> From<TransactionError<E>> for Error {
    fn from(error: TransactionError<E>) -> Self {
        match error {
            TransactionError::Connection(error) => error,
            TransactionError::Transaction(error) => error.into(),
            TransactionError::RollbackFailed { cause, rollback } => Error::RollbackFailed {
                cause: Box::new(cause.into()),
                rollback: Box::new(rollback),
            },
        }
    }
}

// [spec:pgorm:req:conn.tx+3]    BEGIN on the pooled client
impl TransactionTrait for DatabaseConnection {}

impl sealed::Handle for DatabaseConnection {
    type Transaction<'t> = DatabaseTransaction<'t>;
}

impl sealed::Open for DatabaseConnection {
    async fn open(&mut self, _: sealed::Token) -> Result<DatabaseTransaction<'_>, Error> {
        Ok(DatabaseTransaction(Some(self.0.transaction().await?)))
    }
}

// [spec:pgorm:req:conn.tx+3]    a savepoint inside a transaction
impl TransactionTrait for DatabaseTransaction<'_> {}

impl sealed::Handle for DatabaseTransaction<'_> {
    type Transaction<'t> = DatabaseTransaction<'t>;
}

impl sealed::Open for DatabaseTransaction<'_> {
    async fn open(&mut self, _: sealed::Token) -> Result<DatabaseTransaction<'_>, Error> {
        Ok(DatabaseTransaction(Some(
            self.tx_mut().transaction().await?,
        )))
    }
}

// [spec:pgorm:sem:conn.tx.guard+4]    the awaited ends a closure's outcome selects
impl sealed::Close for DatabaseTransaction<'_> {
    // [spec:pgorm:sem:conn.tx.aborted]    a COMMIT or RELEASE an aborted transaction answers is an error
    async fn commit(mut self, _: sealed::Token) -> Result<(), Error> {
        self.take().commit().await.map_err(|error| match error {
            CommitError::Aborted {
                refusal,
                rollback: None,
            } => Error::TransactionAborted(refusal),
            CommitError::Aborted {
                refusal,
                rollback: Some(rollback),
            } => Error::RollbackFailed {
                cause: Box::new(Error::TransactionAborted(refusal)),
                rollback: Box::new(Error::Postgres(rollback)),
            },
            CommitError::Failed(error) => Error::Postgres(error),
        })
    }

    async fn rollback(mut self, _: sealed::Token) -> Result<(), Error> {
        Ok(self.take().rollback().await?)
    }
}
