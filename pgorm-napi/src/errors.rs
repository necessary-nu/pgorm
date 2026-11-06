//! Failures leave Rust as data and become JavaScript errors on the JavaScript
//! thread.
//!
//! A query fails on a runtime thread, where no JavaScript value can be made, so
//! its failure is captured as a [`Failure`] and turned into an error object in
//! the closure that settles the promise. The error classes are JavaScript's:
//! `lib/index.js` defines them and hands the addon a factory that builds one,
//! so an error the addon rejects with is an instance of the class the
//! declarations name.

use std::sync::{Mutex, PoisonError};

use neon::{prelude::*, thread::LocalKey};

/// The factory `lib/index.js` registers: `(kind, message, details) => Error`.
///
/// Instance-local because a class belongs to one JavaScript realm: a worker
/// thread loading the addon registers its own.
static ERROR_FACTORY: LocalKey<Mutex<Option<Root<JsFunction>>>> = LocalKey::new();

/// Connection secrets never enter an error message, nested causes included.
#[derive(Debug, Clone, Default)]
pub(crate) struct Redactions(Vec<String>);

impl Redactions {
    pub(crate) fn from_config(config: &pgorm::Config) -> Self {
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

    fn apply(&self, input: &str) -> String {
        self.0.iter().fold(input.to_owned(), |text, secret| {
            text.replace(secret, "[redacted]")
        })
    }
}

/// The diagnostic fields of an error PostgreSQL reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Diagnostics {
    pub(crate) sqlstate: String,
    pub(crate) message: String,
    pub(crate) severity: String,
    pub(crate) detail: Option<String>,
    pub(crate) hint: Option<String>,
    pub(crate) schema: Option<String>,
    pub(crate) table: Option<String>,
    pub(crate) column: Option<String>,
    pub(crate) datatype: Option<String>,
    pub(crate) constraint: Option<String>,
}

/// Why an operation failed, in the terms the JavaScript error classes carry.
// [spec:pgorm:req:napi.errors+3]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Failure {
    /// PostgreSQL rejected the statement; becomes a `DatabaseError` carrying
    /// the SQLSTATE, or the subclass of it a constraint violation names.
    Database(Box<Diagnostics>),
    /// The server could not be reached, or the connection broke; becomes a
    /// `ConnectionError`.
    Connection(String),
    /// A result could not be decoded into the JavaScript value asked for;
    /// becomes a `DecodeError`.
    Decode(String),
    /// A value could not be sent as the parameter its placeholder declares;
    /// becomes a `ConstructionError`.
    Construction(String),
    /// pgorm or the binding failed in a way no input should cause — a panic
    /// on the runtime; becomes an `InternalError`.
    Internal(String),
    /// A pool, connection, transaction or stream was used after it closed,
    /// or while another operation held it; becomes a `LifecycleError`.
    Lifecycle(String),
    /// Acquiring a connection outlasted its budget; becomes a `TimeoutError`.
    Timeout(String),
    /// The caller's `AbortSignal` fired. `lib/index.js` rejects with the
    /// signal's reason instead.
    Cancelled,
}

impl Failure {
    /// The class name `lib/index.js` maps the failure to.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            // [spec:pgorm:req:bindings.errors+6]    the constraint classes pgorm's SqlError names
            Self::Database(diagnostics) => match diagnostics.sqlstate.as_str() {
                "23505" => "UniqueViolationError",
                "23503" => "ForeignKeyViolationError",
                "23514" => "CheckViolationError",
                "23502" => "NotNullViolationError",
                "23P01" => "ExclusionViolationError",
                _ => "DatabaseError",
            },
            Self::Connection(_) => "ConnectionError",
            Self::Decode(_) => "DecodeError",
            Self::Construction(_) => "ConstructionError",
            Self::Internal(_) => "InternalError",
            Self::Lifecycle(_) => "LifecycleError",
            Self::Timeout(_) => "TimeoutError",
            Self::Cancelled => "CancelledError",
        }
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Database(diagnostics) => &diagnostics.message,
            Self::Connection(message)
            | Self::Decode(message)
            | Self::Construction(message)
            | Self::Internal(message)
            | Self::Lifecycle(message)
            | Self::Timeout(message) => message,
            Self::Cancelled => "the operation was aborted",
        }
    }

    /// The error object a promise rejects with.
    pub(crate) fn into_js<'cx>(self, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        let details = cx.empty_object();
        if let Self::Database(diagnostics) = &self {
            let fields = [
                ("sqlstate", Some(&diagnostics.sqlstate)),
                ("severity", Some(&diagnostics.severity)),
                ("detail", diagnostics.detail.as_ref()),
                ("hint", diagnostics.hint.as_ref()),
                ("schema", diagnostics.schema.as_ref()),
                ("table", diagnostics.table.as_ref()),
                ("column", diagnostics.column.as_ref()),
                ("datatype", diagnostics.datatype.as_ref()),
                ("constraint", diagnostics.constraint.as_ref()),
            ];
            for (name, value) in fields {
                let value: Handle<JsValue> = match value {
                    Some(text) => cx.string(text).upcast(),
                    None => cx.null().upcast(),
                };
                details.prop(cx, name).set(value)?;
            }
        }
        let factory = ERROR_FACTORY.get(cx).and_then(|slot| {
            slot.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
                .map(|root| root.to_inner(cx))
        });
        match factory {
            Some(factory) => factory
                .bind(cx)
                .arg(self.kind())?
                .arg(self.message())?
                .arg(details)?
                .call(),
            // The addon loaded without `lib/index.js`: a plain `Error` named
            // for the class, carrying the same fields.
            None => {
                let error = cx.error(self.message())?;
                error.prop(cx, "name").set(self.kind())?;
                for name in details.get_own_property_names(cx)?.to_vec(cx)? {
                    let name = name.downcast_or_throw::<JsString, _>(cx)?;
                    let value: Handle<JsValue> = details.get(cx, name)?;
                    error.set(cx, name, value)?;
                }
                Ok(error.upcast())
            }
        }
    }
}

/// Throw `failure` as the error class it names.
pub(crate) fn throw<'cx, T>(cx: &mut Cx<'cx>, failure: Failure) -> NeonResult<T> {
    let error = failure.into_js(cx)?;
    cx.throw(error)
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for Failure {}

/// Classify a pgorm error. A PostgreSQL error anywhere in its cause chain
/// decides the class, so a server's refusal during connection (a bad password,
/// a missing database) carries its SQLSTATE like any other.
// [spec:pgorm:req:bindings.errors+6]
pub(crate) fn failure(error: &pgorm::Error, secrets: &Redactions) -> Failure {
    // The server's 25P02 refused the statement pgorm asked with, not the
    // callback's work, so pgorm's account of the rollback is the message.
    // [spec:pgorm:req:bindings.transactions+1]
    if let pgorm::Error::TransactionAborted(refusal) = error {
        return postgres_failure(refusal, Some(&error.to_string()), secrets);
    }
    let mut source: &(dyn std::error::Error + 'static) = error;
    loop {
        if let Some(postgres) = source.downcast_ref::<tokio_postgres::Error>() {
            return postgres_failure(postgres, None, secrets);
        }
        match source.source() {
            Some(next) => source = next,
            None => break,
        }
    }
    match error {
        // [spec:pgorm:req:bindings.pool-hooks]    a timeout names what ran out, and a
        // hook's refusal says why
        pgorm::Error::Pool(error) => {
            match pgorm_bindings::pool_failure(error, pgorm_bindings::Dialect::JavaScript) {
                pgorm_bindings::PoolFailure::Timeout(message) => Failure::Timeout(message),
                pgorm_bindings::PoolFailure::Connection(message) => {
                    Failure::Connection(secrets.apply(&message))
                }
            }
        }
        // A channel name the listener refuses before sending anything is the
        // caller's input, as a parameter that cannot be bound is.
        // [spec:pgorm:req:bindings.listen+3]
        pgorm::Error::Listen(pgorm::ListenError::ChannelTooLong { .. }) => {
            Failure::Construction(secrets.apply(&error.to_string()))
        }
        pgorm::Error::Conversion { .. } | pgorm::Error::Type(_) | pgorm::Error::Json(_) => {
            Failure::Decode(secrets.apply(&error.to_string()))
        }
        // A refusal pgorm-migration's runner makes of the program's
        // migrations — a name twice, a migration missing, a checksum that has
        // changed — as pgorm-python makes it.
        // [spec:pgorm:req:bindings.migrations]
        pgorm::Error::Custom(_) => Failure::Construction(secrets.apply(&error.to_string())),
        // [spec:pgorm:req:conn.copy.statement]
        pgorm::Error::CopyStatement => Failure::Construction(
            "a COPY runs through copyIn or copyOut, which carry its data, not as a statement: \
             give copyIn a copyFrom statement and copyOut a copyTo statement"
                .to_owned(),
        ),
        _ => Failure::Connection(secrets.apply(&error.to_string())),
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
        return if let Some(refusal) = pgorm::DecodeRefusal::of(error) {
            Failure::Decode(secrets.apply(&refusal.to_string()))
        } else if let Some(refusal) = pgorm::BindRefusal::of(error) {
            Failure::Construction(secrets.apply(&refusal.to_string()))
        } else if let Some(refusal) = pgorm::ParameterCountRefusal::of(error) {
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
    }))
}

/// `setErrorFactory(factory)`: register the function failures are built
/// with. `lib/index.js` calls it once as it loads; a later call replaces it.
pub(crate) fn set_error_factory(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    let factory = cx.argument::<JsFunction>(0)?.root(&mut cx);
    let slot = ERROR_FACTORY.get_or_init(&mut cx, Default::default);
    let previous = slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .replace(factory);
    if let Some(previous) = previous {
        previous.drop(&mut cx);
    }
    Ok(cx.undefined())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn a_failure_names_the_class_it_becomes() {
        let database = Failure::Database(Box::new(Diagnostics {
            sqlstate: "22003".into(),
            message: "integer out of range".into(),
            severity: "ERROR".into(),
            detail: None,
            hint: None,
            schema: None,
            table: None,
            column: None,
            datatype: None,
            constraint: None,
        }));
        assert_eq!(database.kind(), "DatabaseError");
        assert_eq!(database.message(), "integer out of range");
        assert_eq!(Failure::Connection("x".into()).kind(), "ConnectionError");
        assert_eq!(Failure::Decode("x".into()).kind(), "DecodeError");
        assert_eq!(
            Failure::Construction("x".into()).kind(),
            "ConstructionError"
        );
        assert_eq!(Failure::Internal("x".into()).kind(), "InternalError");
    }

    // [spec:pgorm:req:bindings.errors+6/test]
    #[test]
    fn a_constraint_violation_names_its_own_class() {
        let violation = |sqlstate: &str| {
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
            }))
        };
        for (sqlstate, kind) in [
            ("23505", "UniqueViolationError"),
            ("23503", "ForeignKeyViolationError"),
            ("23514", "CheckViolationError"),
            ("23502", "NotNullViolationError"),
            ("23P01", "ExclusionViolationError"),
            ("23000", "DatabaseError"),
            ("23001", "DatabaseError"),
        ] {
            assert_eq!(violation(sqlstate).kind(), kind, "{sqlstate}");
        }
    }

    #[test]
    fn a_type_error_is_a_decode_failure() {
        let error = pgorm::Error::Type("not an int4".into());
        assert_eq!(
            failure(&error, &Redactions::default()),
            Failure::Decode("Type Error: not an int4".into())
        );
    }
}
