use pyo3::prelude::*;

use crate::PgormError;

pyo3::create_exception!(pgorm, ConstructionError, PgormError);
pyo3::create_exception!(pgorm, ConnectionError, PgormError);
pyo3::create_exception!(pgorm, DatabaseError, PgormError);
pyo3::create_exception!(pgorm, DecodeError, PgormError);
pyo3::create_exception!(pgorm, TimeoutError, PgormError);
pyo3::create_exception!(pgorm, LifecycleError, PgormError);
pyo3::create_exception!(pgorm, InternalError, PgormError);
pyo3::create_exception!(pgorm, UniqueViolationError, DatabaseError);
pyo3::create_exception!(pgorm, ForeignKeyViolationError, DatabaseError);
pyo3::create_exception!(pgorm, CheckViolationError, DatabaseError);
pyo3::create_exception!(pgorm, NotNullViolationError, DatabaseError);
pyo3::create_exception!(pgorm, ExclusionViolationError, DatabaseError);
pyo3::create_exception!(pgorm, RollbackFailedError, PgormError);

/// Connection secrets must never enter diagnostics, including nested causes.
#[derive(Clone, Default)]
pub(crate) struct Redactions(Vec<String>);

impl Redactions {
    pub(crate) fn from_config(config: &pgorm::Config) -> Self {
        let mut values = Vec::new();
        if let Some(user) = config.get_user().filter(|s| !s.is_empty()) {
            values.push(user.to_owned());
        }
        if let Some(password) = config.get_password().filter(|s| !s.is_empty()) {
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

// [spec:pgorm:req:bindings.errors+5]
pub(crate) fn database_error(
    error: impl std::borrow::Borrow<pgorm::Error>,
    secrets: &Redactions,
) -> PyErr {
    let error = error.borrow();
    if let pgorm::Error::RollbackFailed { cause, rollback } = error {
        return rollback_failed(
            database_error(&**cause, secrets),
            database_error(&**rollback, secrets),
        );
    }
    let retryable = error.is_retryable();
    let raised = classified(error, secrets);
    if retryable {
        let marked = Python::attach(|py| raised.value(py).setattr("is_retryable", true));
        if marked.is_err() {
            return InternalError::new_err("could not mark a retryable error");
        }
    }
    raised
}

/// A transaction whose block failed with `cause`, and whose `ROLLBACK` then
/// failed with `rollback`: Rust's `Error::RollbackFailed`, its two errors as
/// `cause` and `rollback`.
// [spec:pgorm:req:bindings.rollback-failed]
pub(crate) fn rollback_failed(cause: PyErr, rollback: PyErr) -> PyErr {
    Python::attach(|py| {
        let raised = RollbackFailedError::new_err(format!(
            "{}; its ROLLBACK failed: {}",
            cause.value(py),
            rollback.value(py)
        ));
        let decorated = (|| -> PyResult<()> {
            let value = raised.value(py);
            value.setattr("rollback", rollback.value(py))?;
            value.setattr("cause", cause.value(py))?;
            value.setattr("__cause__", cause.value(py))?;
            Ok(())
        })();
        match decorated {
            Ok(()) => raised,
            Err(_) => InternalError::new_err("could not attach a failed rollback's errors"),
        }
    })
}

/// The `RollbackFailedError` a block's exception `cause` and its rollback's
/// failure `rollback` make, for the transaction scopes Python ends.
// [spec:pgorm:req:bindings.rollback-failed]
#[pyfunction(name = "_rollback_failed")]
fn rollback_failed_error<'py>(
    py: Python<'py>,
    cause: Bound<'py, pyo3::exceptions::PyBaseException>,
    rollback: Bound<'py, pyo3::exceptions::PyBaseException>,
) -> Bound<'py, pyo3::exceptions::PyBaseException> {
    rollback_failed(
        PyErr::from_value(cause.into_any()),
        PyErr::from_value(rollback.into_any()),
    )
    .into_value(py)
    .into_bound(py)
}

fn classified(error: &pgorm::Error, secrets: &Redactions) -> PyErr {
    let mut source: &(dyn std::error::Error + 'static) = error;
    loop {
        if let Some(postgres) = source.downcast_ref::<tokio_postgres::Error>() {
            return postgres_error(postgres, secrets);
        }
        match source.source() {
            Some(next) => source = next,
            None => break,
        }
    }
    match error {
        pgorm::Error::Pool(_) => ConnectionError::new_err("PostgreSQL pool acquisition failed"),
        // [spec:pgorm:req:bindings.listen+2]
        pgorm::Error::Listen(pgorm::ListenError::ChannelTooLong { .. }) => {
            ConstructionError::new_err(secrets.apply(&error.to_string()))
        }
        pgorm::Error::Listen(_) => ConnectionError::new_err(secrets.apply(&error.to_string())),
        // A result that is not the row or rows asked for, a `one` finding none
        // among them, cannot become the record asked for.
        // [spec:pgorm:req:bindings.row-count]
        pgorm::Error::Conversion { .. }
        | pgorm::Error::Type(_)
        | pgorm::Error::Json(_)
        | pgorm::Error::RecordNotFound
        | pgorm::Error::RecordNotInserted
        | pgorm::Error::UnpackInsertId
        | pgorm::Error::Verify(_) => DecodeError::new_err(secrets.apply(&error.to_string())),
        // A write pgorm refuses to build from what it was given, and the
        // refusal an application's ActiveModelBehavior hook returns:
        // pgorm-python makes no `Error::Custom` of its own.
        // [spec:pgorm:req:bindings.entity-writes+1]
        pgorm::Error::NothingToSet
        | pgorm::Error::PrimaryKeyNotSet
        | pgorm::Error::AttrNotSet(_)
        | pgorm::Error::QueryBuilder(_)
        | pgorm::Error::Custom(_) => ConstructionError::new_err(secrets.apply(&error.to_string())),
        _ => DatabaseError::new_err(secrets.apply(&error.to_string())),
    }
}

fn postgres_error(error: &tokio_postgres::Error, secrets: &Redactions) -> PyErr {
    let Some(db) = error.as_db_error() else {
        return client_error(error, secrets);
    };
    let message = secrets.apply(db.message());
    // [spec:pgorm:req:bindings.errors+5]    the constraint classes pgorm's SqlError names
    let exception = match db.code().code() {
        "23505" => UniqueViolationError::new_err(message.clone()),
        "23503" => ForeignKeyViolationError::new_err(message.clone()),
        "23514" => CheckViolationError::new_err(message.clone()),
        "23502" => NotNullViolationError::new_err(message.clone()),
        "23P01" => ExclusionViolationError::new_err(message.clone()),
        _ => DatabaseError::new_err(message.clone()),
    };
    let decorated = Python::attach(|py| -> PyResult<()> {
        let value = exception.value(py);
        value.setattr("sqlstate", db.code().code())?;
        value.setattr("message", message)?;
        value.setattr("severity", db.severity())?;
        for (field, content) in [
            ("detail", db.detail()),
            ("hint", db.hint()),
            ("schema", db.schema()),
            ("table", db.table()),
            ("column", db.column()),
            ("datatype", db.datatype()),
            ("constraint", db.constraint()),
        ] {
            value.setattr(field, content.map(|text| secrets.apply(text)))?;
        }
        Ok(())
    });
    match decorated {
        Ok(()) => exception,
        Err(_) => InternalError::new_err("could not attach PostgreSQL diagnostics"),
    }
}

// [spec:pgorm:req:bindings.errors+5]
fn client_error(error: &tokio_postgres::Error, secrets: &Redactions) -> PyErr {
    // Keep the original Rust error intact until this Python boundary so
    // application hooks can still match pgorm::Error::Postgres.
    // [spec:pgorm:req:bindings.refusal-fields]
    let decorated = |raised: PyErr, fields: [(&str, Option<Py<PyAny>>); 2]| {
        Python::attach(|py| {
            let value = raised.value(py);
            for (field, content) in fields {
                if value.setattr(field, content).is_err() {
                    return InternalError::new_err("could not attach a refusal's fields");
                }
            }
            raised
        })
    };
    let text = |content: &Option<String>| {
        content.as_deref().map(|text| {
            Python::attach(|py| {
                pyo3::types::PyString::new(py, &secrets.apply(text))
                    .into_any()
                    .unbind()
            })
        })
    };
    if let Some(refusal) = pgorm::DecodeRefusal::of(error) {
        let reading = Python::attach(|py| {
            pyo3::types::PyString::new(py, &secrets.apply(&refusal.reading))
                .into_any()
                .unbind()
        });
        decorated(
            DecodeError::new_err(secrets.apply(&refusal.to_string())),
            [
                ("reading", Some(reading)),
                ("reason", text(&refusal.reason)),
            ],
        )
    } else if let Some(refusal) = pgorm::BindRefusal::of(error) {
        let parameter =
            Python::attach(|py| pyo3::IntoPyObjectExt::into_py_any(refusal.parameter, py).ok());
        decorated(
            ConstructionError::new_err(secrets.apply(&refusal.to_string())),
            [("parameter", parameter), ("reason", text(&refusal.reason))],
        )
    } else {
        ConnectionError::new_err("PostgreSQL connection or protocol failure")
    }
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = module.py();
    // A DatabaseError raised without the server's answer has no diagnostics.
    // Keep the documented optional fields present on every instance.
    let database_error = py.get_type::<DatabaseError>();
    for field in [
        "sqlstate",
        "message",
        "severity",
        "detail",
        "hint",
        "schema",
        "table",
        "column",
        "datatype",
        "constraint",
    ] {
        database_error.setattr(field, py.None())?;
    }
    // [spec:pgorm:req:bindings.transaction-retry]    a failure is retryable only when pgorm says so
    py.get_type::<crate::PgormError>()
        .setattr("is_retryable", false)?;
    // [spec:pgorm:req:bindings.refusal-fields]
    for field in ["parameter", "reason"] {
        py.get_type::<ConstructionError>()
            .setattr(field, py.None())?;
    }
    for field in ["reading", "reason"] {
        py.get_type::<DecodeError>().setattr(field, py.None())?;
    }
    let rollback_failed = py.get_type::<RollbackFailedError>();
    rollback_failed.setattr("cause", py.None())?;
    rollback_failed.setattr("rollback", py.None())?;
    module.add("RollbackFailedError", rollback_failed)?;
    module.add_function(wrap_pyfunction!(rollback_failed_error, module)?)?;
    module.add("ConstructionError", py.get_type::<ConstructionError>())?;
    module.add("ConnectionError", py.get_type::<ConnectionError>())?;
    module.add("DatabaseError", database_error)?;
    module.add("DecodeError", py.get_type::<DecodeError>())?;
    module.add("TimeoutError", py.get_type::<TimeoutError>())?;
    module.add("LifecycleError", py.get_type::<LifecycleError>())?;
    module.add("InternalError", py.get_type::<InternalError>())?;
    module.add(
        "UniqueViolationError",
        py.get_type::<UniqueViolationError>(),
    )?;
    module.add(
        "ForeignKeyViolationError",
        py.get_type::<ForeignKeyViolationError>(),
    )?;
    module.add("CheckViolationError", py.get_type::<CheckViolationError>())?;
    module.add(
        "NotNullViolationError",
        py.get_type::<NotNullViolationError>(),
    )?;
    module.add(
        "ExclusionViolationError",
        py.get_type::<ExclusionViolationError>(),
    )?;
    module.add(
        "CancelledError",
        py.import("asyncio")?.getattr("CancelledError")?,
    )?;
    Ok(())
}
