//! Why an operation failed, classified once for both bindings.
//!
//! A query fails on the runtime, where no value of either language can be
//! made, so its failure leaves pgorm as data: a [`Failure`] naming the error
//! class `bindings.errors` gives it and carrying the fields that class has.
//! Each binding raises it as its own class of that name, Python's exception or
//! JavaScript's error object, on its own thread.

use std::fmt;

use pgorm::{BindRefusal, DecodeRefusal, Error, ListenError, ParameterCountRefusal};

use crate::{Dialect, pool::pool_failure};

/// Connection secrets, which never enter what a failure says, its nested
/// causes included.
#[derive(Clone, Default)]
pub struct Redactions(Vec<String>);

impl fmt::Debug for Redactions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Redactions").field(&self.0.len()).finish()
    }
}

impl Redactions {
    /// The user and password `config` connects with, the longer first so
    /// that a secret inside another is never left half written.
    pub fn from_config(config: &pgorm::Config) -> Self {
        let mut values = Vec::new();
        if let Some(user) = config.get_user().filter(|user| !user.is_empty()) {
            values.push(user.to_owned());
        }
        if let Some(password) = config
            .get_password()
            .filter(|password| !password.is_empty())
        {
            values.push(String::from_utf8_lossy(password).into_owned());
        }
        values.sort_by_key(|value| std::cmp::Reverse(value.len()));
        Self(values)
    }

    /// `input` with every secret written `[redacted]`.
    pub fn apply(&self, input: &str) -> String {
        self.0.iter().fold(input.to_owned(), |text, secret| {
            text.replace(secret, "[redacted]")
        })
    }
}

/// The diagnostic fields of an error PostgreSQL reported, redacted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostics {
    /// The SQLSTATE, which names the class.
    pub sqlstate: String,
    /// The server's message, or pgorm's account where it replaces it.
    pub message: String,
    /// `ERROR`, `FATAL` or `PANIC`.
    pub severity: String,
    /// `DETAIL`.
    pub detail: Option<String>,
    /// `HINT`.
    pub hint: Option<String>,
    /// The schema of the object the error is about.
    pub schema: Option<String>,
    /// The table the error is about.
    pub table: Option<String>,
    /// The column the error is about.
    pub column: Option<String>,
    /// The data type the error is about, a domain's included.
    pub datatype: Option<String>,
    /// The constraint the error is about.
    pub constraint: Option<String>,
    /// Whether pgorm's `Error::is_retryable` says the transaction is worth
    /// running again: a serialization failure or a deadlock.
    pub retryable: bool,
}

impl Diagnostics {
    /// `DatabaseError`, or the subclass of it a constraint violation names.
    // [spec:pgorm:req:bindings.errors+6]    the constraint classes pgorm's SqlError names
    pub fn class(&self) -> Class {
        match self.sqlstate.as_str() {
            "23505" => Class::UniqueViolation,
            "23503" => Class::ForeignKeyViolation,
            "23514" => Class::CheckViolation,
            "23502" => Class::NotNullViolation,
            "23P01" => Class::ExclusionViolation,
            _ => Class::Database,
        }
    }
}

/// Why an operation failed, in the terms both bindings' error classes carry
/// (`bindings.errors`).
// [spec:pgorm:req:bindings.errors+6]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// PostgreSQL rejected the statement: a `DatabaseError`, or the subclass
    /// of it a constraint violation names.
    Database(Box<Diagnostics>),
    /// The server could not be reached, or the connection broke: a
    /// `ConnectionError`.
    Connection(String),
    /// A result could not become the value asked for: a `DecodeError`.
    Decode(String),
    /// An argument's value cannot become what pgorm sends: a
    /// `ConstructionError`.
    Construction(String),
    /// The driver refused a parameter's value as it encoded it, pgorm's
    /// `BindRefusal`: a `ConstructionError` carrying its fields.
    Bind {
        /// The refusal's text.
        message: String,
        /// The parameter's position, from 1.
        parameter: usize,
        /// The encoder's reason, if it gave one.
        reason: Option<String>,
    },
    /// The driver could not read a result, pgorm's `DecodeRefusal`: a
    /// `DecodeError` carrying its fields.
    Unreadable {
        /// The refusal's text.
        message: String,
        /// What was being read.
        reading: String,
        /// The decoder's reason, if it gave one.
        reason: Option<String>,
    },
    /// pgorm or the binding failed in a way no input should cause: an
    /// `InternalError`.
    Internal(String),
    /// A pool, connection, transaction, stream or listener used after it
    /// closed, or while another operation holds it: a `LifecycleError`.
    Lifecycle(String),
    /// No connection came free in time: a `TimeoutError`.
    Timeout(String),
    /// The caller cancelled the operation, which the binding reports in its
    /// language's own way.
    Cancelled,
    /// pgorm's `Error::RollbackFailed`: a transaction that failed with
    /// `cause`, whose `ROLLBACK` then failed with `rollback`.
    RollbackFailed {
        /// Both failures' messages, joined.
        message: String,
        /// Why the transaction failed.
        cause: Box<Failure>,
        /// Why its `ROLLBACK` then failed.
        rollback: Box<Failure>,
    },
}

/// The class a failure is raised as, by the name both bindings give it,
/// [`Class::name`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Class {
    Database,
    /// SQLSTATE `23505`.
    UniqueViolation,
    /// SQLSTATE `23503`.
    ForeignKeyViolation,
    /// SQLSTATE `23514`.
    CheckViolation,
    /// SQLSTATE `23502`.
    NotNullViolation,
    /// SQLSTATE `23P01`.
    ExclusionViolation,
    Connection,
    Decode,
    Construction,
    Internal,
    Lifecycle,
    Timeout,
    /// The language's own cancellation.
    Cancelled,
    /// A failed transaction whose `ROLLBACK` failed too.
    RollbackFailed,
}

impl Class {
    /// The class's name where both bindings spell it alike: the language's
    /// own cancellation and failed rollback are each binding's to name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Database => "DatabaseError",
            Self::UniqueViolation => "UniqueViolationError",
            Self::ForeignKeyViolation => "ForeignKeyViolationError",
            Self::CheckViolation => "CheckViolationError",
            Self::NotNullViolation => "NotNullViolationError",
            Self::ExclusionViolation => "ExclusionViolationError",
            Self::Connection => "ConnectionError",
            Self::Decode => "DecodeError",
            Self::Construction => "ConstructionError",
            Self::Internal => "InternalError",
            Self::Lifecycle => "LifecycleError",
            Self::Timeout => "TimeoutError",
            Self::Cancelled => "CancelledError",
            Self::RollbackFailed => "RollbackFailedError",
        }
    }
}

impl Failure {
    /// The class the failure is raised as.
    pub fn class(&self) -> Class {
        match self {
            Self::Database(diagnostics) => diagnostics.class(),
            Self::Connection(_) => Class::Connection,
            Self::Decode(_) | Self::Unreadable { .. } => Class::Decode,
            Self::Construction(_) | Self::Bind { .. } => Class::Construction,
            Self::Internal(_) => Class::Internal,
            Self::Lifecycle(_) => Class::Lifecycle,
            Self::Timeout(_) => Class::Timeout,
            Self::Cancelled => Class::Cancelled,
            Self::RollbackFailed { .. } => Class::RollbackFailed,
        }
    }

    /// What the failure says.
    pub fn message(&self) -> &str {
        match self {
            Self::Database(diagnostics) => &diagnostics.message,
            Self::Connection(message)
            | Self::Decode(message)
            | Self::Construction(message)
            | Self::Internal(message)
            | Self::Lifecycle(message)
            | Self::Timeout(message)
            | Self::Bind { message, .. }
            | Self::Unreadable { message, .. }
            | Self::RollbackFailed { message, .. } => message,
            Self::Cancelled => "the operation was aborted",
        }
    }

    /// pgorm's `Error::RollbackFailed` of two classified failures.
    // [spec:pgorm:req:bindings.rollback-failed]
    pub fn rollback_failed(cause: Self, rollback: Self) -> Self {
        Self::RollbackFailed {
            message: format!(
                "{}; its ROLLBACK failed: {}",
                cause.message(),
                rollback.message()
            ),
            cause: Box::new(cause),
            rollback: Box::new(rollback),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for Failure {}

/// The most values one statement binds, as PostgreSQL's protocol counts them
/// in sixteen bits.
pub const MAX_PARAMETERS: usize = 65_535;

/// Refuse a statement binding `count` values, past [`MAX_PARAMETERS`],
/// before anything is sent.
pub fn parameter_count(count: usize) -> Result<(), Failure> {
    match count > MAX_PARAMETERS {
        true => Err(Failure::Construction(format!(
            "the statement binds {count} values, past PostgreSQL's {MAX_PARAMETERS}"
        ))),
        false => Ok(()),
    }
}

/// Classify a pgorm error for the binding `dialect` names. A PostgreSQL error
/// anywhere in its cause chain decides the class, so a server's refusal
/// during connection (a bad password, a missing database) carries its
/// SQLSTATE like any other.
// [spec:pgorm:req:bindings.errors+6]
pub fn classify(error: &Error, secrets: &Redactions, dialect: Dialect) -> Failure {
    let marked = |failure: Failure| match failure {
        Failure::Database(mut diagnostics) => {
            diagnostics.retryable = error.is_retryable();
            Failure::Database(diagnostics)
        }
        other => other,
    };
    if let Error::RollbackFailed { cause, rollback } = error {
        return Failure::rollback_failed(
            classify(cause, secrets, dialect),
            classify(rollback, secrets, dialect),
        );
    }
    // The server's 25P02 refused the statement pgorm asked with, not the
    // block's work, so pgorm's account of the rollback is the message.
    // [spec:pgorm:req:bindings.transactions+2]
    if let Error::TransactionAborted(refusal) = error {
        return marked(postgres_failure(refusal, Some(&error.to_string()), secrets));
    }
    let mut source: &(dyn std::error::Error + 'static) = error;
    loop {
        if let Some(postgres) = source.downcast_ref::<tokio_postgres::Error>() {
            return marked(postgres_failure(postgres, None, secrets));
        }
        match source.source() {
            Some(next) => source = next,
            None => break,
        }
    }
    let message = || secrets.apply(&error.to_string());
    match error {
        // [spec:pgorm:req:bindings.pool-hooks]    a timeout names what ran out, and a
        // hook's refusal says why
        Error::Pool(error) => match pool_failure(error, dialect) {
            crate::pool::PoolFailure::Timeout(message) => Failure::Timeout(message),
            crate::pool::PoolFailure::Connection(message) => {
                Failure::Connection(secrets.apply(&message))
            }
        },
        // A channel name the listener refuses before sending anything is the
        // caller's input, as a parameter that cannot be bound is.
        // [spec:pgorm:req:bindings.listen+3]
        Error::Listen(ListenError::ChannelTooLong { .. }) => Failure::Construction(message()),
        // [spec:pgorm:req:conn.copy.statement]
        Error::CopyStatement => Failure::Construction(format!(
            "a COPY runs through {copy_in} or {copy_out}, which carry its data, not as a \
             statement: {}",
            match dialect {
                Dialect::Python => "give copy_in a CopyFrom and copy_out a CopyTo",
                Dialect::JavaScript =>
                    "give copyIn a copyFrom statement and copyOut a copyTo statement",
            },
            copy_in = dialect.option("copy_in"),
            copy_out = dialect.option("copy_out"),
        )),
        // A result that is not the row or rows asked for, a `one` finding none
        // among them, cannot become the record asked for.
        // [spec:pgorm:req:bindings.row-count]
        Error::Conversion { .. }
        | Error::Type(_)
        | Error::Json(_)
        | Error::RecordNotFound
        | Error::RecordNotInserted
        | Error::UnpackInsertId
        | Error::Verify(_) => Failure::Decode(message()),
        // A write pgorm refuses to build from what it was given, a query it
        // refuses to run as written, and the refusal an application's
        // ActiveModelBehavior hook returns: the bindings make no
        // `Error::Custom` of their own.
        // [spec:pgorm:req:bindings.entity-writes+1]
        Error::NothingToSet
        | Error::PrimaryKeyNotSet
        | Error::AttrNotSet(_)
        | Error::QueryBuilder(_)
        | Error::Query(_)
        | Error::Custom(_) => Failure::Construction(message()),
        _ => Failure::Connection(message()),
    }
}

/// `error` as the failure its SQLSTATE names, its message the server's
/// unless `message` replaces it.
fn postgres_failure(
    error: &tokio_postgres::Error,
    message: Option<&str>,
    secrets: &Redactions,
) -> Failure {
    let Some(db) = error.as_db_error() else {
        // [spec:pgorm:req:bindings.refusal-fields]
        let reason = |reason: &Option<String>| reason.as_deref().map(|text| secrets.apply(text));
        return if let Some(refusal) = DecodeRefusal::of(error) {
            Failure::Unreadable {
                message: secrets.apply(&refusal.to_string()),
                reading: secrets.apply(&refusal.reading),
                reason: reason(&refusal.reason),
            }
        } else if let Some(refusal) = BindRefusal::of(error) {
            Failure::Bind {
                message: secrets.apply(&refusal.to_string()),
                parameter: refusal.parameter,
                reason: reason(&refusal.reason),
            }
        } else if let Some(refusal) = ParameterCountRefusal::of(error) {
            // [spec:pgorm:sem:error.model.parameter-count]
            Failure::Construction(refusal.to_string())
        } else {
            Failure::Connection(format!(
                "PostgreSQL connection or protocol failure: {}",
                secrets.apply(&error.to_string())
            ))
        };
    };
    let optional = |field: Option<&str>| field.map(|text| secrets.apply(text));
    Failure::Database(Box::new(Diagnostics {
        sqlstate: db.code().code().to_owned(),
        message: secrets.apply(message.unwrap_or(db.message())),
        severity: db.severity().to_owned(),
        detail: optional(db.detail()),
        hint: optional(db.hint()),
        schema: optional(db.schema()),
        table: optional(db.table()),
        column: optional(db.column()),
        datatype: optional(db.datatype()),
        constraint: optional(db.constraint()),
        retryable: false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_statement_binds_at_most_the_protocol_limit() {
        assert_eq!(parameter_count(MAX_PARAMETERS), Ok(()));
        assert_eq!(
            parameter_count(MAX_PARAMETERS + 1).map_err(|failure| failure.message().to_owned()),
            Err("the statement binds 65536 values, past PostgreSQL's 65535".to_owned())
        );
    }

    fn diagnostics(sqlstate: &str) -> Failure {
        Failure::Database(Box::new(Diagnostics {
            sqlstate: sqlstate.into(),
            message: "refused".into(),
            severity: "ERROR".into(),
            detail: None,
            hint: None,
            schema: None,
            table: None,
            column: None,
            datatype: None,
            constraint: None,
            retryable: false,
        }))
    }

    #[test]
    fn redactions_remove_the_user_and_password() {
        let config: pgorm::Config = match "postgres://alice:s3cret@localhost/db".parse() {
            Ok(config) => config,
            Err(error) => panic!("the DSN parses: {error}"),
        };
        let secrets = Redactions::from_config(&config);
        assert_eq!(
            secrets.apply("password authentication failed for alice using s3cret"),
            "password authentication failed for [redacted] using [redacted]"
        );
        assert!(!format!("{secrets:?}").contains("s3cret"));
    }

    // [spec:pgorm:req:bindings.rollback-failed/test]    pgorm's own RollbackFailed keeps both errors
    #[test]
    fn a_failed_rollback_keeps_both_failures() {
        let error = Error::RollbackFailed {
            cause: Box::new(Error::Custom("aborted".into())),
            rollback: Box::new(Error::Custom("connection lost".into())),
        };
        assert_eq!(
            classify(&error, &Redactions::default(), Dialect::JavaScript),
            Failure::RollbackFailed {
                message:
                    "Custom Error: aborted; its ROLLBACK failed: Custom Error: connection lost"
                        .into(),
                cause: Box::new(Failure::Construction("Custom Error: aborted".into())),
                rollback: Box::new(Failure::Construction(
                    "Custom Error: connection lost".into()
                )),
            }
        );
    }

    // [spec:pgorm:req:bindings.errors+6/test]    each failure names the class it is raised as, a
    // constraint violation its own subclass
    #[test]
    fn a_failure_names_the_class_it_becomes() {
        for (sqlstate, class) in [
            ("23505", "UniqueViolationError"),
            ("23503", "ForeignKeyViolationError"),
            ("23514", "CheckViolationError"),
            ("23502", "NotNullViolationError"),
            ("23P01", "ExclusionViolationError"),
            ("23000", "DatabaseError"),
            ("22003", "DatabaseError"),
        ] {
            assert_eq!(diagnostics(sqlstate).class().name(), class, "{sqlstate}");
        }
        assert_eq!(diagnostics("22003").message(), "refused");
        for (failure, class) in [
            (Failure::Connection("x".into()), Class::Connection),
            (Failure::Decode("x".into()), Class::Decode),
            (Failure::Construction("x".into()), Class::Construction),
            (Failure::Internal("x".into()), Class::Internal),
            (Failure::Lifecycle("x".into()), Class::Lifecycle),
            (Failure::Timeout("x".into()), Class::Timeout),
        ] {
            assert_eq!(failure.class(), class);
        }
    }

    // [spec:pgorm:req:bindings.errors+6/test]    pgorm's own refusals are classified alike in both
    // bindings: a wrong row count or an unreadable value a DecodeError, a write or query pgorm
    // refuses to build and an application hook's refusal a ConstructionError
    #[test]
    fn pgorm_refusals_are_classified_alike_for_both_bindings() {
        let secrets = Redactions::default();
        for dialect in [Dialect::Python, Dialect::JavaScript] {
            for (error, class) in [
                (Error::Type("not an int4".into()), Class::Decode),
                (Error::RecordNotFound, Class::Decode),
                (Error::RecordNotInserted, Class::Decode),
                (Error::NothingToSet, Class::Construction),
                (Error::PrimaryKeyNotSet, Class::Construction),
                (
                    Error::Custom("before_save refused".into()),
                    Class::Construction,
                ),
                (
                    Error::Query(pgorm::RuntimeError::Internal("empty".into())),
                    Class::Construction,
                ),
                (Error::CopyStatement, Class::Construction),
            ] {
                assert_eq!(
                    classify(&error, &secrets, dialect).class(),
                    class,
                    "{error}"
                );
            }
        }
        assert_eq!(
            classify(&Error::CopyStatement, &secrets, Dialect::Python).message(),
            "a COPY runs through copy_in or copy_out, which carry its data, not as a statement: \
             give copy_in a CopyFrom and copy_out a CopyTo"
        );
        assert_eq!(
            classify(&Error::CopyStatement, &secrets, Dialect::JavaScript).message(),
            "a COPY runs through copyIn or copyOut, which carry its data, not as a statement: \
             give copyIn a copyFrom statement and copyOut a copyTo statement"
        );
    }
}
