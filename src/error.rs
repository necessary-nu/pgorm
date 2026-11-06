use tokio_postgres::error::SqlState;

/// An error from unsuccessful database operations
// [spec:pgorm:def:error.model+14]
#[derive(thiserror::Error, Debug)]
#[doc(alias = "DbErr")]
#[non_exhaustive]
pub enum Error {
    /// Postgres error
    #[error("Postgres Error: {0:?} {db:?}", db = .0.as_db_error())]
    Postgres(#[from] tokio_postgres::Error),
    /// Pool error
    #[error("Pool Error: {0}")]
    Pool(#[from] pgorm_pool::PoolError),
    /// Runtime type conversion error
    #[error("Error converting `{from}` into `{into}`: {source}")]
    Conversion {
        /// From type
        from: &'static str,
        /// Into type
        into: &'static str,
        /// The conversion failure being reported
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// An error occurred while performing a query
    #[error("Query Error: {0}")]
    Query(#[source] RuntimeError),
    /// The inserted row's primary key could not be decoded from `RETURNING`
    #[error("Failed to unpack the inserted primary key")]
    UnpackInsertId,
    /// A primary-key column of the model is `NotSet`, so there is nothing to
    /// narrow the statement to a single row
    #[error("A primary key value is not set")]
    PrimaryKeyNotSet,
    /// Thrown by `TryFrom<ActiveModel>`, which assumes all attributes are set/unchanged
    #[error("Attribute {0} is NotSet")]
    AttrNotSet(String),
    /// Error occurred while parsing value as target type
    #[error("Type Error: {0}")]
    Type(String),
    /// Error occurred while parsing json value as target type
    #[error("Json Error: {0}")]
    Json(String),
    /// The record was not found in the database
    #[error("No records were returned for the given query")]
    RecordNotFound,
    /// None of the records are inserted,
    /// that probably means all of them conflict with existing records in the table
    #[error("None of the records are inserted")]
    RecordNotInserted,
    /// The update names no column to set, so there is no statement to send.
    /// Distinct from an update whose `WHERE` matched no row, which is `Ok(0)`
    #[error("The update has no columns to set")]
    NothingToSet,
    /// A decode target does not match the statement it would decode
    #[error("Verification Error: {0}")]
    Verify(#[from] VerifyError),
    /// The statement builder rejected its input
    #[error("Query Builder Error: {0}")]
    QueryBuilder(#[from] pgorm_query::error::Error),
    /// A listener refused a channel name, or stopped its notifications
    #[error("Listen Error: {0}")]
    Listen(#[from] ListenError),
    /// What `?` makes of a transaction's
    /// [`TransactionError::RollbackFailed`](crate::TransactionError::RollbackFailed):
    /// its closure failed with `cause`, and the `ROLLBACK` that followed
    /// failed with `rollback`.
    ///
    /// How that transaction ended is unknown. A savepoint's is settled by the
    /// transaction it was nested in, whose own rollback discards it; a
    /// transaction opened on a connection leaves the connection's state
    /// unknown, so [`discard`](crate::DatabaseConnection::discard) it.
    #[error("{cause}; its ROLLBACK failed: {rollback}")]
    RollbackFailed {
        /// The closure's own error, which is why the transaction failed
        #[source]
        cause: Box<Error>,
        /// What the `ROLLBACK` failed with
        rollback: Box<Error>,
    },
    /// A transaction's closure returned `Ok`, but a statement that failed
    /// inside it, its error caught by the closure, had aborted the
    /// transaction, so it could only roll back: it was rolled back, and
    /// nothing it wrote stands.
    ///
    /// A savepoint's is rolled back to, so the transaction it was nested in
    /// carries on. The connection is in a known state either way. The source
    /// is the server's refusal of the statement pgorm sends ahead of the
    /// `COMMIT` or `RELEASE` to ask, SQLSTATE `25P02`
    /// (`in_failed_sql_transaction`).
    #[error(
        "the transaction was rolled back, not committed: a statement that failed inside it had aborted it"
    )]
    TransactionAborted(#[source] tokio_postgres::Error),
    /// SQL text holding a `COPY` was given to a statement method, which runs
    /// what it is given as a statement; a `COPY`'s data flows beside it, so it
    /// runs only through [`CopyTrait`](crate::CopyTrait)'s `copy_in` and
    /// `copy_out`. Nothing was sent.
    #[error(
        "a COPY runs through CopyTrait's copy_in or copy_out, which carry its data, not as a statement"
    )]
    CopyStatement,
    /// A custom error
    #[error("Custom Error: {0}")]
    Custom(String),
}

/// Why a `FromQueryResult` target does not match the statement it would decode,
/// as reported by [`VerifyStatement::verify`](crate::VerifyStatement::verify).
///
/// Each variant names the target, the column at fault, and what the statement
/// says about that column, so the message identifies the field to change
/// without a second lookup.
// [spec:pgorm:req:exec.verify.errors]
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum VerifyError {
    /// The target reads a column the statement does not return
    #[error(
        "`{target}` reads column `{column}`, which the statement does not return; it returns: {returned}"
    )]
    ColumnMissing {
        /// The decode target being verified
        target: &'static str,
        /// The column name the target looks up
        column: &'static str,
        /// The column names the statement does return, in order
        returned: String,
    },
    /// The statement returns the column, but its PostgreSQL type is not one the
    /// field's Rust type can decode
    #[error(
        "`{target}` decodes column `{column}` as `{rust_type}`, which cannot read PostgreSQL type `{pg_type}`"
    )]
    ColumnType {
        /// The decode target being verified
        target: &'static str,
        /// The column name the target looks up
        column: &'static str,
        /// The Rust type the target decodes the column into
        rust_type: &'static str,
        /// The PostgreSQL type the statement returns for that column
        pg_type: String,
    },
    /// The target reports no columns, so there is nothing to check it against
    #[error(
        "`{target}` reports no columns: a hand-written `FromQueryResult` impl that does not override `expected_columns` cannot be verified"
    )]
    Unreflected {
        /// The decode target being verified
        target: &'static str,
    },
}

/// Why a [`Listener`](crate::Listener) refused a channel name, or why its
/// [`Notifications`](crate::Notifications) ended.
// [spec:pgorm:req:conn.listen]
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ListenError {
    /// The channel name is longer than the 63 bytes PostgreSQL keeps of an
    /// identifier, so the server would listen on a truncated name instead
    #[error(
        "channel name `{channel}` is longer than the 63 bytes PostgreSQL keeps of an identifier"
    )]
    ChannelTooLong {
        /// The name as given
        channel: String,
    },
    /// Another notification arrived while `capacity` were waiting unread, so
    /// the listener closed its connection rather than drop one
    #[error(
        "a notification arrived while {capacity} were waiting unread; the listener closed its connection"
    )]
    Overflow {
        /// How many notifications the stream holds unread
        capacity: usize,
    },
}

/// The result of a fallible pgorm operation, defaulting to [`Error`].
///
/// The default type parameter lets callers write `pgorm::Result<Model>` for
/// the common case while still naming a foreign error type — as
/// [`DatabaseConnection::transaction_with`](crate::DatabaseConnection::transaction_with)
/// does — where the closure carries its own.
///
/// ```
/// fn first_cake_id() -> pgorm::Result<i32> {
///     Err(pgorm::Error::RecordNotFound)
/// }
///
/// assert!(first_cake_id().is_err());
/// ```
// [spec:pgorm:def:error.model+14]    crate-root Result alias
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Runtime error
// [spec:pgorm:def:error.model.runtime+3]
#[derive(thiserror::Error, Debug)]
pub enum RuntimeError {
    /// Error generated from within pgorm
    #[error("{0}")]
    Internal(String),
}

// [spec:pgorm:def:error.model+14]    Display-string equality
impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}

impl Eq for Error {}

/// Error during `impl FromStr for Entity::Column`
#[derive(thiserror::Error, Debug)]
#[error("Failed to match \"{0}\" as Column")]
pub struct ColumnFromStrError(pub String);

#[allow(dead_code)]
pub(crate) fn query_err<T>(s: T) -> Error
where
    T: ToString,
{
    Error::Query(RuntimeError::Internal(s.to_string()))
}

#[allow(dead_code)]
pub(crate) fn type_err<T>(s: T) -> Error
where
    T: ToString,
{
    Error::Type(s.to_string())
}

#[allow(dead_code)]
pub(crate) fn json_err<T>(s: T) -> Error
where
    T: ToString,
{
    Error::Json(s.to_string())
}

/// A constraint violation PostgreSQL reported, classified by its SQLSTATE.
///
/// Every variant carries the server's message and the names it reported
/// alongside it, in a [`ConstraintViolation`].
// [spec:pgorm:sem:error.model.sql-class+4]
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
#[doc(alias = "SqlErr")]
pub enum SqlError {
    /// A duplicate key in a unique index or primary key (`23505`)
    #[error("Unique Constraint Violated: {}", .0.message)]
    UniqueConstraintViolation(ConstraintViolation),
    /// A foreign key with no matching referenced row, or a referenced row
    /// still in use (`23503`)
    #[error("Foreign Key Constraint Violated: {}", .0.message)]
    ForeignKeyConstraintViolation(ConstraintViolation),
    /// A table's or a domain's `CHECK` constraint refused a value (`23514`)
    #[error("Check Constraint Violated: {}", .0.message)]
    CheckConstraintViolation(ConstraintViolation),
    /// A null in a `NOT NULL` column, or in a domain that does not allow
    /// null values (`23502`)
    #[error("Not-Null Constraint Violated: {}", .0.message)]
    NotNullConstraintViolation(ConstraintViolation),
    /// A row conflicting with another under an exclusion constraint, which
    /// includes a `WITHOUT OVERLAPS` primary key or unique key (`23P01`)
    #[error("Exclusion Constraint Violated: {}", .0.message)]
    ExclusionConstraintViolation(ConstraintViolation),
}

/// What PostgreSQL reported about a constraint violation: its message, and
/// the object names the server attached to it.
///
/// Which names are present depends on what refused the value. A table's
/// constraint names its `schema`, `table` and `constraint`; a table's
/// `NOT NULL` column names its `schema`, `table` and `column` but no
/// constraint; a domain's `CHECK` names its `schema`, the domain as
/// `datatype`, and the `constraint`; a domain's `NOT NULL` names only the
/// `schema` and `datatype`.
// [spec:pgorm:sem:error.model.sql-class+4]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConstraintViolation {
    /// The server's primary message
    pub message: String,
    /// The schema of the table or domain at fault
    pub schema: Option<String>,
    /// The table the violated constraint or column belongs to
    pub table: Option<String>,
    /// The column a `NOT NULL` violation is in
    pub column: Option<String>,
    /// The domain whose constraint refused the value
    pub datatype: Option<String>,
    /// The name of the violated constraint
    pub constraint: Option<String>,
}

impl SqlError {
    // [spec:pgorm:sem:error.model.sql-class+4]    SQLSTATE to variant
    fn classify(db_error: &tokio_postgres::error::DbError) -> Option<Self> {
        let variant = match *db_error.code() {
            SqlState::UNIQUE_VIOLATION => Self::UniqueConstraintViolation,
            SqlState::FOREIGN_KEY_VIOLATION => Self::ForeignKeyConstraintViolation,
            SqlState::CHECK_VIOLATION => Self::CheckConstraintViolation,
            SqlState::NOT_NULL_VIOLATION => Self::NotNullConstraintViolation,
            SqlState::EXCLUSION_VIOLATION => Self::ExclusionConstraintViolation,
            _ => return None,
        };
        Some(variant(ConstraintViolation {
            message: db_error.message().to_owned(),
            schema: db_error.schema().map(str::to_owned),
            table: db_error.table().map(str::to_owned),
            column: db_error.column().map(str::to_owned),
            datatype: db_error.datatype().map(str::to_owned),
            constraint: db_error.constraint().map(str::to_owned),
        }))
    }
}

/// A value the driver refused to encode for one of a statement's parameters,
/// while building the bind message and so before anything reached the
/// server: which parameter, and why.
///
/// Its `Display` is `parameter $N cannot be bound: <reason>`, the text each
/// language binding reports the refusal with.
// [spec:pgorm:sem:error.model.bind-refusal]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BindRefusal {
    /// The one-based number of the parameter's `$N` placeholder.
    pub parameter: usize,
    /// The codec's refusal of the value — pgorm's own for a `Value` the
    /// parameter's type cannot take — where the driver kept one.
    pub reason: Option<String>,
}

impl BindRefusal {
    /// The refusal `error` is, or `None` for any other failure.
    // [spec:pgorm:sem:error.model.bind-refusal]
    pub fn of(error: &tokio_postgres::Error) -> Option<Self> {
        // tokio-postgres 0.7.18 has no public kind for an encode failure. Its
        // top-level text names the phase and the zero-based position, and is
        // read rather than a cause, whose text a caller's value can shape.
        let index = error
            .to_string()
            .strip_prefix("error serializing parameter ")?
            .parse::<usize>()
            .ok()?;
        Some(Self {
            parameter: index + 1,
            reason: std::error::Error::source(error).map(ToString::to_string),
        })
    }
}

impl std::fmt::Display for BindRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "parameter ${} cannot be bound", self.parameter)?;
        match &self.reason {
            Some(reason) => write!(f, ": {reason}"),
            None => Ok(()),
        }
    }
}

/// A statement given another number of parameters than it takes, which the
/// driver refuses once the statement is prepared and before it binds
/// anything: how many its `$N` placeholders take, and how many were given.
///
/// Its `Display` is `the statement takes N parameters, and M were given`, the
/// text each language binding reports the refusal with.
// [spec:pgorm:sem:error.model.parameter-count]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ParameterCountRefusal {
    /// How many parameters the statement takes.
    pub expected: usize,
    /// How many were given.
    pub given: usize,
}

impl ParameterCountRefusal {
    /// The refusal `error` is, or `None` for any other failure.
    // [spec:pgorm:sem:error.model.parameter-count]
    pub fn of(error: &tokio_postgres::Error) -> Option<Self> {
        // As for BindRefusal: tokio-postgres 0.7.18 has no public kind for
        // it, and its top-level text, which no value shapes, is read.
        let counts = error.to_string();
        let (expected, given) = counts
            .strip_prefix("expected ")?
            .split_once(" parameters but got ")?;
        Some(Self {
            expected: expected.parse().ok()?,
            given: given.parse().ok()?,
        })
    }
}

impl std::fmt::Display for ParameterCountRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let takes = if self.expected == 1 {
            "parameter"
        } else {
            "parameters"
        };
        let were = if self.given == 1 { "was" } else { "were" };
        write!(
            f,
            "the statement takes {} {takes}, and {} {were} given",
            self.expected, self.given
        )
    }
}

/// A result the driver could not read into what was asked for: a column's
/// value its decoder refused, a column the row does not have, or a row or a
/// row count of another shape than the statement promised.
///
/// Its `Display` is the driver's account of what it was reading, followed by
/// the decoder's reason where there is one, the text each language binding
/// reports the failure with.
// [spec:pgorm:sem:error.model.decode-refusal]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeRefusal {
    /// The driver's account of what it was reading: `error deserializing
    /// column 0`, ``invalid column `name` ``, or the unexpected number of
    /// columns or rows a query returned.
    pub reading: String,
    /// The decoder's refusal of the value, where the driver kept one.
    pub reason: Option<String>,
}

impl DecodeRefusal {
    /// The refusal `error` is, or `None` for any other failure.
    // [spec:pgorm:sem:error.model.decode-refusal]
    pub fn of(error: &tokio_postgres::Error) -> Option<Self> {
        // As for BindRefusal: the phase is read off the driver's own
        // top-level text, never off a cause a value can shape.
        let reading = error.to_string();
        let decoding = reading.starts_with("error deserializing column ")
            || reading.starts_with("invalid column `")
            || reading == "query returned an unexpected number of columns"
            || reading == "query returned an unexpected number of rows";
        decoding.then(|| Self {
            reason: std::error::Error::source(error).map(ToString::to_string),
            reading,
        })
    }
}

impl std::fmt::Display for DecodeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reading)?;
        match &self.reason {
            Some(reason) => write!(f, ": {reason}"),
            None => Ok(()),
        }
    }
}

/// An error after which re-running the whole transaction may succeed.
///
/// Implemented for [`Error`]; implement it for a domain error type to use that
/// type as the closure error of
/// [`DatabaseConnection::transaction_with_retry`](crate::DatabaseConnection::transaction_with_retry).
// [spec:pgorm:sem:conn.tx.retry+3]    retryability predicate
pub trait RetryableError {
    /// Whether the transaction that produced this error is worth retrying.
    fn is_retryable(&self) -> bool;
}

// [spec:pgorm:sem:conn.tx.retry+3]
impl RetryableError for Error {
    fn is_retryable(&self) -> bool {
        Error::is_retryable(self)
    }
}

impl Error {
    /// Whether this is a transaction-rollback error PostgreSQL expects the
    /// client to retry: SQLSTATE `40001` (`serialization_failure`) or `40P01`
    /// (`deadlock_detected`).
    // [spec:pgorm:sem:conn.tx.retry+3]    SQLSTATE classification
    pub fn is_retryable(&self) -> bool {
        let Error::Postgres(error) = self else {
            return false;
        };
        let Some(db_error) = error.as_db_error() else {
            return false;
        };
        let code = db_error.code();
        *code == SqlState::T_R_SERIALIZATION_FAILURE || *code == SqlState::T_R_DEADLOCK_DETECTED
    }

    /// The value a statement's parameter refused, when this is that failure:
    /// [`BindRefusal::of`] the driver's error, and `None` for any other
    /// variant.
    // [spec:pgorm:sem:error.model.bind-refusal]
    pub fn bind_refusal(&self) -> Option<BindRefusal> {
        match self {
            Error::Postgres(e) => BindRefusal::of(e),
            _ => None,
        }
    }

    /// The parameters a statement was given another number of than it takes,
    /// when this is that failure: [`ParameterCountRefusal::of`] the driver's
    /// error, and `None` for any other variant.
    // [spec:pgorm:sem:error.model.parameter-count]
    pub fn parameter_count_refusal(&self) -> Option<ParameterCountRefusal> {
        match self {
            Error::Postgres(e) => ParameterCountRefusal::of(e),
            _ => None,
        }
    }

    /// The result the driver could not read, when this is that failure:
    /// [`DecodeRefusal::of`] the driver's error, and `None` for any other
    /// variant.
    // [spec:pgorm:sem:error.model.decode-refusal]
    pub fn decode_refusal(&self) -> Option<DecodeRefusal> {
        match self {
            Error::Postgres(e) => DecodeRefusal::of(e),
            _ => None,
        }
    }

    /// Classify an [`Error`] as a [`SqlError`], returning `None` when the error is
    /// not a recognised constraint violation.
    // [spec:pgorm:sem:error.model.sql-class+4]    classifier entry point
    #[doc(alias = "sql_err")]
    pub fn sql_error(&self) -> Option<SqlError> {
        match self {
            Error::Postgres(e) => SqlError::classify(e.as_db_error()?),
            _ => None,
        }
    }
}
