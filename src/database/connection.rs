use crate::Error;
use pgorm_driver::{
    Row, RowStream,
    types::{BorrowToSql, ToSql},
};

pub(super) mod private {
    pub trait Sealed {
        #[cfg(feature = "turso")]
        fn native_parameter_types(
            &self,
        ) -> Result<Vec<pgorm_driver::types::Type>, pgorm_driver::Error> {
            Ok(Vec::new())
        }
    }

    impl Sealed for str {}
    impl Sealed for String {}
}

/// SQL text accepted by a [`ConnectionTrait`] statement method.
///
/// Callers supply `str` or `String`. The sealed trait also admits an internal
/// wrapper that retains ORM parameter types for experimental native execution.
/// Every form carries SQL text, with no prepared connection handle.
/// tokio-postgres bounds its own statement surface on
/// `ToStatement`, which additionally admits an already-prepared `Statement` —
/// and a `Statement` names a statement that exists on the one connection it was
/// prepared against, so handing it to another connection compiles and then
/// fails on the wire with SQLSTATE `26000`, *prepared statement does not exist*.
/// Bounding on this trait instead deletes that call rather than diagnosing it.
///
/// Because every statement retains its text, [`sql_text`](SqlText::sql_text)
/// always has an answer. The two readers of it — the statement cache
/// (`conn.pool.conn-trait`) and the metrics fingerprint (`metric.fingerprint`)
/// — are therefore total: neither has a case where the SQL it is looking at has
/// gone missing.
// [spec:pgorm:def:conn.sql-text+4]
#[doc(alias("Statement", "StatementBuilder", "from_sql_and_values", "from_string"))]
pub trait SqlText: private::Sealed {
    /// The statement's SQL text.
    fn sql_text(&self) -> &str;
}

// [spec:pgorm:def:conn.sql-text+4]    the two spellings of a statement
impl SqlText for str {
    fn sql_text(&self) -> &str {
        self
    }
}

impl SqlText for String {
    fn sql_text(&self) -> &str {
        self
    }
}

/// The generic API for a database connection that can perform query or execute statements.
/// It abstracts database connection and transaction
///
/// The six statement methods resolve their SQL through the connection's
/// prepared-statement cache. PostgreSQL uses the extended protocol; native
/// Turso uses the embedded execution API and includes parameter types in the
/// cache key. The seventh, [`batch_execute`], runs an uncached script.
///
/// A statement is [`SqlText`] — `&str` or `&String` — rather than
/// tokio-postgres's `ToStatement`, so a `Statement` prepared elsewhere is not
/// something a call site can reach for:
///
/// ```compile_fail,E0277
/// # use pgorm::{ConnectionTrait, DatabaseConnection, Error};
/// # use pgorm_driver::Statement;
/// async fn run(db: &DatabaseConnection, prepared: &Statement) -> Result<u64, Error> {
///     db.execute(prepared, &[]).await
/// }
/// ```
///
/// [`batch_execute`]: ConnectionTrait::batch_execute
// [spec:pgorm:def:conn.pool.conn-trait+11]
#[async_trait::async_trait]
pub trait ConnectionTrait: Sync {
    /// Execute a SQL statement with its bound parameters, returning the number
    /// of rows it affected
    async fn execute<T>(&self, statement: &T, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync;

    /// [`ConnectionTrait::execute`] over parameters supplied as an iterator of
    /// individually-typed values rather than a slice of one trait object type
    async fn execute_raw<T, P, I>(&self, statement: &T, params: I) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator;

    /// Execute a SQL statement with its bound parameters, expecting exactly one
    /// row; any other row count is an error
    async fn query_one<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Row, Error>
    where
        T: ?Sized + SqlText + Sync;

    /// Execute a SQL statement with its bound parameters, expecting at most one
    /// row; more than one is an error
    async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, Error>
    where
        T: ?Sized + SqlText + Sync;

    /// Execute a SQL statement with its bound parameters, buffering every row
    /// it returns
    async fn query_all<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, Error>
    where
        T: ?Sized + SqlText + Sync;

    /// Execute a statement and return its rows as a stream, without buffering
    /// the whole result set
    // [spec:pgorm:def:exec.stream+3]
    #[doc(alias = "StreamTrait")]
    async fn query_raw<T, P, I>(&self, statement: &T, params: I) -> Result<RowStream, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator;

    /// Run the statements in `sql` as an uncached script.
    ///
    /// PostgreSQL uses the simple-query protocol; native Turso uses its
    /// embedded parser and execution API.
    ///
    /// This is the only method that accepts a multi-statement string: the
    /// others go through the extended protocol, where a prepared statement
    /// carries exactly one command and PostgreSQL answers a second one with
    /// *cannot insert multiple commands into a prepared statement*. Execution
    /// stops at the first statement that fails, and outside an explicit
    /// transaction PostgreSQL runs the whole string as one implicit
    /// transaction, so that failure discards the statements before it too.
    /// Native Turso runs the statements sequentially and can retain effects
    /// applied before a failure outside an explicit transaction.
    ///
    /// It takes no parameters and yields no rows. Nothing is prepared, so the
    /// statement cache is bypassed by construction. It is meant for DDL,
    /// migration, and fixture surfaces, where a script is the unit of work.
    ///
    /// Because there is no parameter binding, any value has to be interpolated
    /// into `sql` by the caller — build these strings from trusted input only,
    /// and reach for [`ConnectionTrait::execute`] the moment user data is
    /// involved.
    #[doc(alias = "execute_unprepared")]
    async fn batch_execute(&self, sql: &str) -> Result<(), Error>;
}

/// A transaction closure is given its handle as `&mut`: `&mut
/// DatabaseTransaction` on a connection or a transaction, `&mut
/// InstrumentedTransaction` on the metrics wrappers. A helper taking `&C`
/// would otherwise have to be passed a reborrowed `&*txn`, since `&mut T` is
/// its own type and the coercion to `&T` does not fire into the inference
/// variable `C`. Forwarding through the exclusive borrow, for every
/// implementor at once, makes each closure's handle spell a connection the way
/// the transaction it borrows does.
// [spec:pgorm:def:conn.pool.conn-trait+11]    the closure's handle is itself a connection
#[async_trait::async_trait]
impl<C: ConnectionTrait> ConnectionTrait for &mut C {
    async fn execute<T>(&self, statement: &T, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        (**self).execute(statement, params).await
    }

    async fn execute_raw<T, P, I>(&self, statement: &T, params: I) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        (**self).execute_raw(statement, params).await
    }

    async fn query_one<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Row, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        (**self).query_one(statement, params).await
    }

    async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        (**self).query_opt(statement, params).await
    }

    async fn query_all<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        (**self).query_all(statement, params).await
    }

    async fn query_raw<T, P, I>(&self, statement: &T, params: I) -> Result<RowStream, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        (**self).query_raw(statement, params).await
    }

    async fn batch_execute(&self, sql: &str) -> Result<(), Error> {
        (**self).batch_execute(sql).await
    }
}
