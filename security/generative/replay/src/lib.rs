//! A standalone replay harness for the pgorm generated-program campaign.
//!
//! This crate is the **subject** side of `security/generative/REPLAY.md`, and
//! only the subject. A generated `main.rs` links it, connects to one database,
//! applies the program's declared fixture, constructs the program's builders,
//! runs its effects, and prints one executor-shaped report on stdout:
//!
//! ```json
//! {"program_sha256": "...", "status": "executed",
//!  "steps": [{"id": "s0", "status": "observed",
//!             "native_paths": ["pgorm::pipeline::Pipeline::filter"],
//!             "observation": {"kind": "rows", "rows": []}}],
//!  "cleanup_errors": [], "builds": 0}
//! ```
//!
//! The independent oracle stays in Python. Writing a second reference
//! implementation in Rust would produce two oracles that can agree with each
//! other and both be wrong, so there is no comparison logic here at all — only
//! lossless recording, in shapes the Python side already reads.
//!
//! Nothing in the dependency graph is Python. Linking libpython into the
//! subject would make the binary's behaviour a property of the binding under
//! test, which is the thing a standalone replay exists to avoid.

pub mod codecs;
mod decode;
pub mod entities;
pub mod observe;
mod report;
pub mod stream;
pub mod wire;

use std::fmt;

pub use report::Report;
pub use tokio_postgres::Row;

use pgorm::{Config, ConnectionTrait, DatabasePool};

/// The environment variable consulted when no URL is given on the command line.
pub const URL_VARIABLE: &str = "DATABASE_URL";

/// A portable artifact is malformed or outside its declared limits.
///
/// The Rust counterpart of `wire.FormatError`: raised by the value encoder, the
/// row decoder, and [`wire::validate`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatError(String);

impl FormatError {
    /// Build an error with this message.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// The message, without the wrapping.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for FormatError {}

/// Why the harness could not do what it was asked.
#[derive(Debug)]
pub enum Error {
    /// The database rejected, or could not be reached for, the operation.
    Database(pgorm::Error),
    /// A value could not be encoded or decoded without loss.
    Format(FormatError),
    /// The harness was invoked without a usable connection URL.
    Configuration(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "{error}"),
            Self::Format(error) => write!(formatter, "{error}"),
            Self::Configuration(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Format(error) => Some(error),
            Self::Configuration(_) => None,
        }
    }
}

impl From<pgorm::Error> for Error {
    fn from(error: pgorm::Error) -> Self {
        Self::Database(error)
    }
}

impl From<FormatError> for Error {
    fn from(error: FormatError) -> Self {
        Self::Format(error)
    }
}

/// A reproducer reaches `tokio_postgres` directly wherever pgorm hands it a
/// driver type — `Row::try_get` for a result reference, `RowStream` for a
/// streamed step — so those failures have to classify as the database failures
/// they are rather than stall at the crate boundary.
impl From<tokio_postgres::Error> for Error {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Database(pgorm::Error::Postgres(error))
    }
}

/// Connection secrets must never enter diagnostics, including nested causes.
///
/// The report is written to stdout and retained with the finding, so the
/// password in a `DATABASE_URL` has to be removed at the point a message is
/// built rather than filtered downstream.
#[derive(Clone, Debug, Default)]
pub struct Redactions(Vec<String>);

impl Redactions {
    /// The user and password a configuration carries, longest first so that a
    /// password containing the user name is still fully replaced.
    pub fn from_config(config: &Config) -> Self {
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

    /// Replace every known secret in the text.
    pub fn apply(&self, input: &str) -> String {
        self.0.iter().fold(input.to_owned(), |text, secret| {
            text.replace(secret, "[redacted]")
        })
    }
}

/// A failure as the campaign records it: a class, a cause, and a SQLSTATE.
///
/// The class names mirror the Python binding's exception types exactly, because
/// the two reports are compared field by field. A cause that reaches a
/// PostgreSQL server error carries that error's message and code; everything
/// else is classified from the same phase prefixes the binding matches on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedError {
    class: String,
    cause: String,
    sqlstate: Option<String>,
}

impl ObservedError {
    /// Build an error observation directly.
    pub fn new(class: &str, cause: impl Into<String>, sqlstate: Option<String>) -> Self {
        Self {
            class: class.to_owned(),
            cause: cause.into(),
            sqlstate,
        }
    }

    /// Classify a pgorm error with no secrets to remove.
    pub fn from_pgorm(error: &pgorm::Error) -> Self {
        Self::classify(error, &Redactions::default())
    }

    /// Classify a lossless-representation failure.
    pub fn from_format(error: &FormatError) -> Self {
        Self::new("DecodeError", error.message(), None)
    }

    /// Classify any harness error.
    pub fn from_error(error: &Error) -> Self {
        Self::classify_error(error, &Redactions::default())
    }

    /// Classify a pgorm error, redacting connection secrets from its message.
    pub fn classify(error: &pgorm::Error, secrets: &Redactions) -> Self {
        let mut source: &(dyn std::error::Error + 'static) = error;
        loop {
            if let Some(postgres) = source.downcast_ref::<tokio_postgres::Error>() {
                return Self::from_postgres(postgres, secrets);
            }
            match source.source() {
                Some(next) => source = next,
                None => break,
            }
        }
        match error {
            pgorm::Error::Pool(_) => Self::new(
                "ConnectionError",
                "PostgreSQL pool acquisition failed",
                None,
            ),
            pgorm::Error::Conversion { .. } | pgorm::Error::Type(_) | pgorm::Error::Json(_) => {
                Self::new("DecodeError", secrets.apply(&error.to_string()), None)
            }
            _ => Self::new("DatabaseError", secrets.apply(&error.to_string()), None),
        }
    }

    /// Classify any harness error, redacting connection secrets.
    pub fn classify_error(error: &Error, secrets: &Redactions) -> Self {
        match error {
            Error::Database(error) => Self::classify(error, secrets),
            Error::Format(error) => Self::from_format(error),
            Error::Configuration(message) => {
                Self::new("ConnectionError", secrets.apply(message), None)
            }
        }
    }

    /// The exception class the Python binding would have raised.
    pub fn class(&self) -> &str {
        &self.class
    }

    /// The failure's message, already redacted.
    pub fn cause(&self) -> &str {
        &self.cause
    }

    /// The five-character SQLSTATE, when the server supplied one.
    pub fn sqlstate(&self) -> Option<&str> {
        self.sqlstate.as_deref()
    }

    fn from_postgres(error: &tokio_postgres::Error, secrets: &Redactions) -> Self {
        let Some(db) = error.as_db_error() else {
            // Pinned tokio-postgres 0.7.18 exposes no public error-kind accessor
            // for FromSql/ToSql failures. Its top-level Display identifies the
            // phase; inspect that before redaction, never a nested cause whose
            // content the program under test chose.
            let phase = error.to_string();
            if phase.starts_with("error deserializing column ")
                || phase.starts_with("invalid column `")
                || phase == "query returned an unexpected number of columns"
            {
                return Self::new("DecodeError", secrets.apply(&phase), None);
            }
            if let Some(refusal) = pgorm::BindRefusal::of(error) {
                return Self::new(
                    "ConstructionError",
                    secrets.apply(&refusal.to_string()),
                    None,
                );
            }
            return Self::new(
                "ConnectionError",
                "PostgreSQL connection or protocol failure",
                None,
            );
        };
        Self::new(
            "DatabaseError",
            secrets.apply(db.message()),
            Some(db.code().code().to_owned()),
        )
    }
}

/// One database, one pool, and the fixture applied to it.
// [spec:pgorm:req:generative.replay]
#[derive(Debug)]
pub struct Harness {
    pool: DatabasePool,
    redactions: Redactions,
}

impl Harness {
    /// The connection URL from `argv[1]`, falling back to `$DATABASE_URL`.
    ///
    /// Credentials are never compiled in: a replay is run against whichever
    /// throwaway database the operator points it at.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Configuration`] when neither source supplies a URL.
    pub fn url_from_environment() -> Result<String, Error> {
        std::env::args()
            .nth(1)
            .or_else(|| std::env::var(URL_VARIABLE).ok())
            .filter(|url| !url.is_empty())
            .ok_or_else(|| {
                Error::Configuration(format!(
                    "no database URL: pass one as the first argument or set {URL_VARIABLE}"
                ))
            })
    }

    /// Connect to the database the URL names.
    ///
    /// The pool is deliberately small — a replay is one program on one
    /// connection, and the campaign's Python executor bounds its own pools the
    /// same way — and a connection is taken once here so an unreachable
    /// database fails before any step is attributed to it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Configuration`] when the URL is not a valid PostgreSQL
    /// configuration, and [`Error::Database`] when the pool cannot be built or
    /// the server cannot be reached.
    pub async fn connect(url: &str) -> Result<Self, Error> {
        let config: Config = url.parse().map_err(|error| {
            // The parse error can quote the URL, password included.
            let _ = error;
            Error::Configuration(
                "connection URL is not a valid PostgreSQL configuration".to_owned(),
            )
        })?;
        let redactions = Redactions::from_config(&config);
        let pool = pgorm::connect_with_builder(config, |builder| builder.max_size(2))?;
        pool.get().await?;
        Ok(Self { pool, redactions })
    }

    /// Apply one multi-statement fixture script.
    ///
    /// Fixture SQL is rendered by `pgorm_campaign.baseline` as a single string
    /// of `;`-separated statements, which is exactly what the simple-query
    /// protocol takes; every other statement method would answer the second
    /// command with *cannot insert multiple commands into a prepared
    /// statement*.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Database`] when any statement in the script fails.
    pub async fn apply(&self, sql: &str) -> Result<(), Error> {
        let connection = self.pool.get().await?;
        connection.batch_execute(sql).await?;
        Ok(())
    }

    /// The pool every effect runs against.
    pub fn pool(&self) -> &DatabasePool {
        &self.pool
    }

    /// The secrets this harness removes from any message it reports.
    pub fn redactions(&self) -> &Redactions {
        &self.redactions
    }

    /// Classify an error for reporting, with this connection's secrets removed.
    pub fn observe(&self, error: &Error) -> ObservedError {
        ObservedError::classify_error(error, &self.redactions)
    }
}

#[cfg(test)]
mod tests;
