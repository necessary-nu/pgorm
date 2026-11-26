use std::{fmt, marker::PhantomData, ops::Deref};

use crate::{Error, ReadOnly};
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

    /// Seals [`RunsText`](super::RunsText).
    pub trait Text {}
}

/// SQL text accepted by a [`Connection`] statement method.
///
/// Callers supply `str` or `String`, or the [`Sql`] that
/// [`sql!`](crate::sql) and [`prql!`](crate::prql) expand to. With the
/// experimental `turso` feature, `ValueSql` also retains borrowed parameter
/// values for native typing. Every form carries SQL text, with no prepared
/// connection handle. tokio-postgres bounds its own statement surface on
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
// [spec:pgorm:def:conn.sql-text+7]
#[doc(alias("StatementBuilder", "from_sql_and_values", "from_string"))]
pub trait SqlText: private::Sealed {
    /// What the text is known to do, which decides the connections it runs
    /// on ([`RunsOn`]): [`Reads`] for a statement `sql!` or pgorm itself
    /// found only reads, [`Writes`] for everything else, text a program
    /// holds only at run time included.
    type Kind;

    /// The statement's SQL text.
    fn sql_text(&self) -> &str;
}

// [spec:pgorm:def:conn.sql-text+7]
impl SqlText for str {
    type Kind = Writes;

    fn sql_text(&self) -> &str {
        self
    }
}

impl SqlText for String {
    type Kind = Writes;

    fn sql_text(&self) -> &str {
        self
    }
}

/// The [`SqlText::Kind`] of a statement known to only read: one `sql!` or
/// `prql!` classified as a read as the crate compiled, or one pgorm built
/// to read. It runs on every [`Connection`].
// [spec:pgorm:def:read-only.kinds]
#[derive(Debug)]
pub enum Reads {}

/// The [`SqlText::Kind`] of a statement that may write: one `sql!` or
/// `prql!` found to write, and any text known only at run time. It runs on
/// a [`ConnectionMut`], and through a read-only connection's `unchecked()`,
/// whose server refuses a write.
// [spec:pgorm:def:read-only.kinds]
#[derive(Debug)]
pub enum Writes {}

/// The connections a statement of this [`SqlText::Kind`] runs on: [`Reads`]
/// on every [`Connection`], [`Writes`] on every [`RunsText`] connection.
///
/// It bounds each [`Connection`] statement method, so run-time text given
/// to a connection that may only read is a compile error naming the
/// connection.
// [spec:pgorm:req:read-only.diagnostics]
#[diagnostic::on_unimplemented(
    message = "`{C}` may only read, and this SQL text is not known to read",
    label = "text written at run time, or a `sql!` statement that may write",
    note = "`sql!` checks a literal as the crate compiles; a statement it finds only reads (a SELECT, VALUES, TABLE, SHOW, or EXPLAIN without ANALYZE, with no data-modifying WITH and no row lock) runs on any `Connection`",
    note = "other text runs on a `ConnectionMut`, a connection that may write, or through a read-only connection's `unchecked()`, whose server refuses a write"
)]
pub trait RunsOn<C: ?Sized> {}

// [spec:pgorm:req:read-only.text]
impl<C: ?Sized + Connection> RunsOn<C> for Reads {}

// [spec:pgorm:req:read-only.text]
#[diagnostic::do_not_recommend]
impl<C: ?Sized + RunsText> RunsOn<C> for Writes {}

/// A connection SQL text not known to read runs on: every
/// [`ConnectionMut`], which may write, and the
/// [`Unchecked`](crate::Unchecked) a read-only connection's `unchecked()`
/// returns, whose server refuses a write. The trait is sealed.
// [spec:pgorm:req:read-only.text]
pub trait RunsText: private::Text {}

impl<C: ?Sized + ConnectionMut> private::Text for C {}
impl<C: ?Sized + ConnectionMut> RunsText for C {}

/// SQL text whose kind pgorm states rather than the text's type: the
/// statements pgorm builds to read, a raw selector's text under the kind it
/// was given, and a statement a wrapping connection hands on to the one it
/// wraps, the wrapper's own bound having admitted it.
pub(crate) struct Kinded<'a, T: ?Sized, K>(&'a T, PhantomData<fn() -> K>);

impl<'a, T: ?Sized, K> Kinded<'a, T, K> {
    pub(crate) fn new(text: &'a T) -> Self {
        Self(text, PhantomData)
    }
}

impl<'a, T: ?Sized> Kinded<'a, T, Reads> {
    /// `text` handed to a wrapped connection as a read. Sound only where the
    /// wrapper is no more able to write than the connection it wraps, so that
    /// its own bound admitted no text the wrapped one's would refuse.
    pub(crate) fn read(text: &'a T) -> Self {
        Self(text, PhantomData)
    }
}

impl<T: ?Sized + private::Sealed, K> private::Sealed for Kinded<'_, T, K> {
    #[cfg(feature = "turso")]
    fn native_parameter_types(
        &self,
    ) -> Result<Vec<pgorm_driver::types::Type>, pgorm_driver::Error> {
        self.0.native_parameter_types()
    }
}

impl<T: ?Sized + SqlText, K> SqlText for Kinded<'_, T, K> {
    type Kind = K;

    fn sql_text(&self) -> &str {
        self.0.sql_text()
    }
}

/// A statement [`sql!`](crate::sql) or [`prql!`](crate::prql) checked as
/// the crate compiled, typed by what it does: `Sql<Reads>` when every
/// statement in it only reads, `Sql<Writes>` otherwise.
///
/// It is [`SqlText`] whose [`Kind`](SqlText::Kind) is that type, so a read
/// runs on every [`Connection`] and a write only where text that may write
/// does. It dereferences to `str`, and [`as_str`](Sql::as_str) gives the
/// text in a `const`:
///
/// ```
/// use pgorm::{Reads, Sql, sql};
///
/// const BY_ID: &Sql<Reads> = sql!("SELECT id, name FROM cake WHERE id = $1");
/// const SCHEMA: &str = sql!("CREATE TABLE cake (id int PRIMARY KEY)").as_str();
/// assert!(BY_ID.starts_with("SELECT"));
/// assert!(SCHEMA.starts_with("CREATE"));
/// ```
// [spec:pgorm:def:read-only.sql-macro]
pub struct Sql<K> {
    text: &'static str,
    kind: PhantomData<fn() -> K>,
}

impl<K> Sql<K> {
    /// What `sql!` and `prql!` expand to, having classified `text`. Calling
    /// it by hand claims a classification nothing checked.
    #[doc(hidden)]
    pub const fn __checked(text: &'static str) -> Self {
        Self {
            text,
            kind: PhantomData,
        }
    }

    /// The statement's text.
    pub const fn as_str(&self) -> &'static str {
        self.text
    }
}

impl<K> Clone for Sql<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for Sql<K> {}

impl<K> fmt::Debug for Sql<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.text, f)
    }
}

impl<K> fmt::Display for Sql<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text)
    }
}

impl<K> PartialEq<str> for Sql<K> {
    fn eq(&self, other: &str) -> bool {
        self.text == other
    }
}

impl<K> PartialEq<Sql<K>> for str {
    fn eq(&self, other: &Sql<K>) -> bool {
        self == other.text
    }
}

impl<K> Deref for Sql<K> {
    type Target = str;

    fn deref(&self) -> &str {
        self.text
    }
}

impl<K> private::Sealed for Sql<K> {}

// [spec:pgorm:def:read-only.sql-macro]
impl<K> SqlText for Sql<K> {
    type Kind = K;

    fn sql_text(&self) -> &str {
        self.text
    }
}

/// What every connection does that only reads: the statement methods, and
/// through them every read pgorm builds — the select, graph and selector
/// terminals, counts, pagination, cursors, pipeline terminals and loaders.
///
/// Writes need [`ConnectionMut`], which every connection that may write
/// implements beside this. A function taking `&impl Connection` cannot
/// write through what it is given, whatever that is: a write pgorm builds
/// needs `ConnectionMut`, and SQL text is held to its [`SqlText::Kind`].
/// What no type shows — a `SELECT` calling a function that writes, a row
/// lock a select builder adds — runs as a read, and only a session the
/// server holds read-only, a [`ReadOnlyConnection`](crate::ReadOnlyConnection)'s,
/// refuses it.
///
/// The six statement methods resolve their SQL through the connection's
/// prepared-statement cache. PostgreSQL uses the extended protocol; native
/// Turso uses the embedded execution API and includes parameter types in the
/// cache key. Each takes its statement where its kind [`RunsOn`] the
/// connection: a [`Reads`] statement anywhere, text that may write only on a
/// connection that may run it.
///
/// A statement is [`SqlText`] — `&str`, `&String` or a `sql!` statement —
/// rather than tokio-postgres's `ToStatement`, so a `Statement` prepared
/// elsewhere is not something a call site can reach for:
///
/// ```compile_fail,E0277
/// # use pgorm::{Connection, DatabaseConnection, Error};
/// # use pgorm_driver::Statement;
/// async fn run(db: &DatabaseConnection, prepared: &Statement) -> Result<u64, Error> {
///     db.execute(prepared, &[]).await
/// }
/// ```
///
/// Nor does text run on a connection known only to read:
///
/// ```compile_fail,E0277
/// # use pgorm::{Connection, Error};
/// async fn run(db: &impl Connection) -> Result<u64, Error> {
///     db.execute("DELETE FROM cake", &[]).await
/// }
/// ```
// [spec:pgorm:def:conn.pool.conn-trait+13]
// [spec:pgorm:def:read-only.traits]
#[async_trait::async_trait]
pub trait Connection: Sync {
    /// Execute a SQL statement with its bound parameters, returning the number
    /// of rows it affected
    async fn execute<T>(&self, statement: &T, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
        T::Kind: RunsOn<Self>;

    /// [`Connection::execute`] over parameters supplied as an iterator of
    /// individually-typed values rather than a slice of one trait object type
    async fn execute_raw<T, P, I>(&self, statement: &T, params: I) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
        T::Kind: RunsOn<Self>,
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
        T: ?Sized + SqlText + Sync,
        T::Kind: RunsOn<Self>;

    /// Execute a SQL statement with its bound parameters, expecting at most one
    /// row; more than one is an error
    async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
        T::Kind: RunsOn<Self>;

    /// Execute a SQL statement with its bound parameters, buffering every row
    /// it returns
    async fn query_all<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
        T::Kind: RunsOn<Self>;

    /// Execute a statement and return its rows as a stream, without buffering
    /// the whole result set
    // [spec:pgorm:def:exec.stream+4]
    #[doc(alias = "StreamTrait")]
    async fn query_raw<T, P, I>(&self, statement: &T, params: I) -> Result<RowStream, Error>
    where
        T: ?Sized + SqlText + Sync,
        T::Kind: RunsOn<Self>,
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
    /// and reach for [`Connection::execute`] the moment user data is
    /// involved.
    ///
    /// It is a method of every connection and runs only on one that may
    /// write, a [`ConnectionMut`]: on any other it is a compile error naming
    /// the connection.
    #[doc(alias = "execute_unprepared")]
    async fn batch_execute(&self, sql: &str) -> Result<(), Error>
    where
        Self: ConnectionMut,
    {
        self.run_batch(sql).await
    }
}

/// A [`Connection`] that may change the database: inserts, updates and
/// deletes, the ActiveModel and Model write methods, migrations, SQL text of
/// any kind, and [`batch_execute`](Connection::batch_execute)'s scripts.
///
/// `DatabaseConnection`, `DatabaseTransaction`, `&DatabaseConnection`,
/// [`HookConnection`](crate::HookConnection), [`DatabaseRef`](crate::DatabaseRef)
/// and the metrics wrappers around a connection that may write implement it.
/// The methods borrow shared, as [`Connection`]'s do: "Mut" says what the
/// connection may do to the database, not to the handle.
// [spec:pgorm:def:read-only.traits]
// [spec:pgorm:req:read-only.diagnostics]
#[diagnostic::on_unimplemented(
    message = "`{Self}` may only read, so it cannot run this write",
    label = "this writes, which needs a `ConnectionMut`",
    note = "a `DatabaseConnection` or `DatabaseTransaction`, and the metrics wrappers around them, may write; a `ReadOnlyConnection`, a `ReadOnlyTransaction`, a connection's `read_only()`, and code generic over `Connection` may only read"
)]
#[async_trait::async_trait]
pub trait ConnectionMut: Connection {
    /// The script [`batch_execute`](Connection::batch_execute) runs, on
    /// this connection. Call `batch_execute`; this is what each connection
    /// that may write implements it with.
    #[doc(hidden)]
    async fn run_batch(&self, sql: &str) -> Result<(), Error>;

    /// This connection, borrowed as one that may only read: a
    /// [`ReadOnly`] runs every read and no write, by its type alone. The
    /// server is not asked; nothing turns it back into a connection that may
    /// write.
    // [spec:pgorm:req:read-only.downgrade]
    fn read_only(&self) -> ReadOnly<'_, Self>
    where
        Self: Sized,
    {
        ReadOnly::new(self)
    }
}

/// A transaction closure is given its handle as `&mut`: `&mut
/// DatabaseTransaction` on a connection or a transaction, `&mut
/// InstrumentedTransaction` on the metrics wrappers. A helper taking `&C`
/// would otherwise have to be passed a reborrowed `&*txn`, since `&mut T` is
/// its own type and the coercion to `&T` does not fire into the inference
/// variable `C`. Forwarding through the exclusive borrow, for every
/// implementor at once, makes each closure's handle spell a connection the way
/// the transaction it borrows does. `&mut C` may run what `C` may, so each
/// statement goes on as a read once this impl's own bound has admitted it.
// [spec:pgorm:def:conn.pool.conn-trait+13]    the closure's handle is itself a connection
#[async_trait::async_trait]
impl<C: Connection> Connection for &mut C {
    async fn execute<T>(&self, statement: &T, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
        T::Kind: RunsOn<Self>,
    {
        (**self).execute(&Kinded::read(statement), params).await
    }

    async fn execute_raw<T, P, I>(&self, statement: &T, params: I) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
        T::Kind: RunsOn<Self>,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        (**self).execute_raw(&Kinded::read(statement), params).await
    }

    async fn query_one<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Row, Error>
    where
        T: ?Sized + SqlText + Sync,
        T::Kind: RunsOn<Self>,
    {
        (**self).query_one(&Kinded::read(statement), params).await
    }

    async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
        T::Kind: RunsOn<Self>,
    {
        (**self).query_opt(&Kinded::read(statement), params).await
    }

    async fn query_all<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
        T::Kind: RunsOn<Self>,
    {
        (**self).query_all(&Kinded::read(statement), params).await
    }

    async fn query_raw<T, P, I>(&self, statement: &T, params: I) -> Result<RowStream, Error>
    where
        T: ?Sized + SqlText + Sync,
        T::Kind: RunsOn<Self>,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        (**self).query_raw(&Kinded::read(statement), params).await
    }
}

// [spec:pgorm:def:read-only.traits]
#[async_trait::async_trait]
impl<C: ConnectionMut> ConnectionMut for &mut C {
    async fn run_batch(&self, sql: &str) -> Result<(), Error> {
        (**self).run_batch(sql).await
    }
}
