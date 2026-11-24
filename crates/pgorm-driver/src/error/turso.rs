use super::{DbError, Error, Kind, SqlState};
use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;

impl DbError {
    // [spec:pgorm:req:turso.errors]
    fn turso(cause: turso_pg::Error) -> Self {
        Self {
            // The native API reports failures without a severity diagnostic.
            severity: "ERROR".to_owned(),
            parsed_severity: None,
            code: SqlState::from_code(cause.sqlstate),
            message: cause.message.clone(),
            detail: cause.detail.clone(),
            hint: cause.hint.clone(),
            position: None,
            where_: None,
            schema: None,
            table: cause.table_name.clone(),
            column: cause.column_name.clone(),
            datatype: None,
            constraint: cause.constraint_name.clone(),
            file: None,
            line: None,
            routine: None,
            native_cause: Some(Arc::new(cause)),
        }
    }
}

// Equality remains a comparison of diagnostics, independent of the backend or
// the identity of its retained cause. This also preserves PostgreSQL equality
// when the optional native backend is compiled in.
impl PartialEq for DbError {
    fn eq(&self, other: &Self) -> bool {
        self.severity == other.severity
            && self.parsed_severity == other.parsed_severity
            && self.code == other.code
            && self.message == other.message
            && self.detail == other.detail
            && self.hint == other.hint
            && self.position == other.position
            && self.where_ == other.where_
            && self.schema == other.schema
            && self.table == other.table
            && self.column == other.column
            && self.datatype == other.datatype
            && self.constraint == other.constraint
            && self.file == other.file
            && self.line == other.line
            && self.routine == other.routine
    }
}

impl Eq for DbError {}

/// A native operation failure followed by a failure to clean up its connection.
///
/// This is the source of the returned driver [`Error`]. Its own source follows
/// the primary failure; [`Self::cleanup`] exposes the separate cleanup failure.
/// The outer error's [`Error::as_db_error`] and [`Error::code`] retain the primary
/// failure's database diagnostics.
#[derive(Debug)]
pub struct TursoCleanupError {
    primary: Error,
    cleanup: Error,
}

impl TursoCleanupError {
    /// Returns the failure that required cleanup.
    pub fn primary(&self) -> &Error {
        &self.primary
    }

    /// Returns the additional failure encountered during cleanup.
    pub fn cleanup(&self) -> &Error {
        &self.cleanup
    }
}

impl fmt::Display for TursoCleanupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}; native cleanup also failed: {}",
            self.primary, self.cleanup
        )?;
        if let Some(source) = self.cleanup.source() {
            write!(formatter, ": {source}")?;
        }
        Ok(())
    }
}

impl StdError for TursoCleanupError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&self.primary)
    }
}

impl Error {
    /// Whether a native statement's prepared metadata is no longer valid.
    ///
    /// Discard the cached statement before another call. This error can be
    /// detected after an engine step and does not establish that the operation
    /// had no effects; it is not permission to retry automatically.
    pub fn is_statement_invalidated(&self) -> bool {
        if let Some(cleanup) = self
            .source()
            .and_then(|cause| cause.downcast_ref::<TursoCleanupError>())
        {
            return cleanup.primary().is_statement_invalidated();
        }
        self.0.kind == Kind::TursoStatementInvalidated
    }

    pub(crate) fn turso_statement_invalidated() -> Self {
        Self::new(Kind::TursoStatementInvalidated, None)
    }

    pub(crate) fn turso(cause: turso_pg::Error) -> Self {
        Self::new(Kind::Db, Some(Box::new(DbError::turso(cause))))
    }

    pub(crate) fn turso_worker<E>(cause: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        Self::new(Kind::TursoWorker, Some(Box::new(cause)))
    }

    pub(crate) fn turso_unsupported(operation: &'static str) -> Self {
        Self::new(Kind::TursoUnsupported(operation), None)
    }

    pub(crate) fn turso_unsupported_with<E>(operation: &'static str, cause: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        Self::new(Kind::TursoUnsupported(operation), Some(Box::new(cause)))
    }

    pub(crate) fn turso_cleanup(primary: Error, cleanup: Error) -> Self {
        Self::new(
            Kind::TursoCleanup,
            Some(Box::new(TursoCleanupError { primary, cleanup })),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{BufMut, BytesMut};
    use postgres_protocol::message::backend::Message;
    use std::io;
    use turso_core::{ConstraintKind, ConstraintViolation, DialectError, LimboError};

    fn dialect_error(code: &'static str, message: &str) -> turso_pg::Error {
        LimboError::Dialect(Box::new(DialectError {
            code,
            message: message.to_owned(),
            detail: Some("native detail".to_owned()),
            hint: Some("native hint".to_owned()),
        }))
        .into()
    }

    #[test]
    // [spec:pgorm:req:turso.errors/test]
    fn native_diagnostics_and_original_cause_are_preserved() {
        // An unknown SQLSTATE must survive too; do not classify from message text.
        let error = Error::turso(dialect_error("ZP123", "native message"));
        let database = error.as_db_error().unwrap();
        assert_eq!(error.code().unwrap().code(), "ZP123");
        assert_eq!(database.message(), "native message");
        assert_eq!(database.detail(), Some("native detail"));
        assert_eq!(database.hint(), Some("native hint"));
        assert_eq!(database.table(), None);
        assert_eq!(database.column(), None);
        assert_eq!(database.constraint(), None);
        assert_eq!(database.schema(), None);
        assert_eq!(database.datatype(), None);
        assert_eq!(database.position(), None);
        assert_eq!(database.where_(), None);
        assert_eq!(database.file(), None);
        assert_eq!(database.line(), None);
        assert_eq!(database.routine(), None);
        assert_eq!(database.parsed_severity(), None);
        let native = database
            .source()
            .unwrap()
            .downcast_ref::<turso_pg::Error>()
            .unwrap();
        assert_eq!(native.sqlstate, "ZP123");
        assert!(native.source().unwrap().is::<LimboError>());
        assert!(!error.is_closed());
    }

    #[test]
    // [spec:pgorm:req:turso.errors/test]
    fn native_constraint_fields_are_retained() {
        let mut violation = ConstraintViolation::new(ConstraintKind::NotNull, "missing".to_owned());
        violation.table_name = Some("widgets".to_owned());
        violation.column_names = vec!["name".to_owned()];
        violation.constraint_name = Some("required_name".to_owned());
        let native: turso_pg::Error = LimboError::ConstraintViolation(Box::new(violation)).into();
        let error = Error::turso(native);
        let database = error.as_db_error().unwrap();
        assert_eq!(error.code(), Some(&SqlState::NOT_NULL_VIOLATION));
        assert_eq!(database.table(), Some("widgets"));
        assert_eq!(database.column(), Some("name"));
        assert_eq!(database.constraint(), Some("required_name"));
    }

    #[test]
    fn postgres_errors_keep_their_source_and_diagnostic_equality() {
        let body = b"SERROR\0CZP123\0Mnative message\0Dnative detail\0Hnative hint\0\0";
        let mut bytes = BytesMut::new();
        bytes.put_u8(b'E');
        bytes.put_u32(u32::try_from(body.len() + 4).unwrap());
        bytes.extend_from_slice(body);
        let Some(Message::ErrorResponse(body)) = Message::parse(&mut bytes).unwrap() else {
            panic!("expected a PostgreSQL error response");
        };
        let postgres = Error::db(body);
        let native = Error::turso(dialect_error("ZP123", "native message"));
        let pg_database = postgres.as_db_error().unwrap();
        let native_database = native.as_db_error().unwrap();
        assert_eq!(pg_database, native_database);
        assert!(pg_database.source().is_none());
        assert!(native_database.source().is_some());
        assert_eq!(native_database, &native_database.clone());
        assert_ne!(
            native_database,
            Error::turso(dialect_error("ZP124", "native message"))
                .as_db_error()
                .unwrap()
        );
    }

    #[test]
    // [spec:pgorm:req:turso.errors/test]
    fn worker_and_unsupported_errors_have_no_sqlstate() {
        let error = Error::turso_worker(io::Error::other("worker stopped"));
        assert!(error.as_db_error().is_none());
        assert!(error.code().is_none());
        assert!(error.source().unwrap().is::<io::Error>());
        assert_eq!(error.source().unwrap().to_string(), "worker stopped");
        let error = Error::turso_unsupported("COPY FROM");
        assert!(error.as_db_error().is_none());
        assert!(error.source().is_none());
        assert!(error.to_string().contains("COPY FROM"));
    }

    #[test]
    // [spec:pgorm:req:turso.errors/test]
    fn cleanup_keeps_both_causes_and_primary_database_code() {
        let primary = Error::turso(dialect_error("23505", "duplicate key"));
        let cleanup = Error::turso(dialect_error("58030", "cleanup I/O failed"));
        let error = Error::turso_cleanup(primary, cleanup);
        assert_eq!(error.code(), Some(&SqlState::UNIQUE_VIOLATION));
        assert_eq!(error.as_db_error().unwrap().message(), "duplicate key");
        let both = error
            .source()
            .unwrap()
            .downcast_ref::<TursoCleanupError>()
            .unwrap();
        assert_eq!(both.primary().code(), Some(&SqlState::UNIQUE_VIOLATION));
        assert_eq!(both.cleanup().code().unwrap().code(), "58030");
        assert!(both.source().unwrap().is::<Error>());
        assert!(both.to_string().contains("cleanup I/O failed"));
        let owned = error.into_source().unwrap();
        let both = owned.downcast_ref::<TursoCleanupError>().unwrap();
        assert_eq!(
            both.cleanup().as_db_error().unwrap().message(),
            "cleanup I/O failed"
        );
    }

    #[test]
    fn cleanup_does_not_substitute_a_secondary_database_code() {
        let error = Error::turso_cleanup(
            Error::closed(),
            Error::turso(dialect_error("58030", "cleanup failed")),
        );
        assert!(error.as_db_error().is_none());
        assert!(error.code().is_none());
        assert!(error.is_closed());
    }

    // [spec:pgorm:req:turso.errors/test]
    #[test]
    fn invalidated_metadata_has_no_fabricated_sqlstate() {
        let invalidated = Error::turso_statement_invalidated();
        assert!(invalidated.is_statement_invalidated());
        assert!(invalidated.code().is_none());
        assert!(invalidated.as_db_error().is_none());
        assert!(!invalidated.is_closed());
        let cleanup = Error::turso_cleanup(invalidated, Error::closed());
        assert!(cleanup.is_statement_invalidated());
        assert!(cleanup.is_closed());
        assert!(
            !Error::turso(dialect_error("0A000", "unsupported feature")).is_statement_invalidated()
        );
    }
}
