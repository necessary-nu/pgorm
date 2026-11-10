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

pub(crate) use pgorm_bindings::{Failure, Redactions};

/// What pgorm-bindings refuses, raised as the exception its class names.
pub(crate) trait Raise<T> {
    fn py(self) -> PyResult<T>;
}

impl<T> Raise<T> for Result<T, Failure> {
    fn py(self) -> PyResult<T> {
        self.map_err(raised)
    }
}

// [spec:pgorm:req:bindings.errors+6]
// [spec:pgorm:sem:error.model.parameter-count]
// [spec:pgorm:req:conn.copy.statement]
pub(crate) fn database_error(
    error: impl std::borrow::Borrow<pgorm::Error>,
    secrets: &Redactions,
) -> PyErr {
    raised(pgorm_bindings::classify(
        error.borrow(),
        secrets,
        crate::config::DIALECT,
    ))
}

/// `failure` as the exception of the class it names, carrying the fields
/// that class has.
// [spec:pgorm:req:bindings.errors+6]
pub(crate) fn raised(failure: Failure) -> PyErr {
    match failure {
        Failure::RollbackFailed {
            cause, rollback, ..
        } => rollback_failed(raised(*cause), raised(*rollback)),
        Failure::Database(diagnostics) => database_raised(*diagnostics),
        Failure::Connection(message) => ConnectionError::new_err(message),
        Failure::Decode(message) => DecodeError::new_err(message),
        Failure::Construction(message) => ConstructionError::new_err(message),
        Failure::Internal(message) => InternalError::new_err(message),
        Failure::Lifecycle(message) => LifecycleError::new_err(message),
        Failure::Timeout(message) => TimeoutError::new_err(message),
        Failure::Cancelled => pyo3::exceptions::asyncio::CancelledError::new_err(
            Failure::Cancelled.message().to_owned(),
        ),
        // [spec:pgorm:req:bindings.refusal-fields]
        Failure::Bind {
            message,
            parameter,
            reason,
        } => decorated(ConstructionError::new_err(message), |value| {
            value.setattr("parameter", parameter)?;
            value.setattr("reason", reason)
        }),
        Failure::Unreadable {
            message,
            reading,
            reason,
        } => decorated(DecodeError::new_err(message), |value| {
            value.setattr("reading", reading)?;
            value.setattr("reason", reason)
        }),
    }
}

/// `raised` with the fields `decorate` sets, or an `InternalError` saying
/// they could not be set.
fn decorated(
    raised: PyErr,
    decorate: impl for<'py> FnOnce(&Bound<'py, pyo3::exceptions::PyBaseException>) -> PyResult<()>,
) -> PyErr {
    Python::attach(|py| match decorate(raised.value(py)) {
        Ok(()) => raised,
        Err(_) => InternalError::new_err("could not attach a failure's fields"),
    })
}

/// A server's error as the class its SQLSTATE names, with its diagnostics,
/// marked retryable when pgorm says the transaction is worth running again.
fn database_raised(diagnostics: pgorm_bindings::Diagnostics) -> PyErr {
    let message = diagnostics.message.clone();
    let exception = match diagnostics.class() {
        pgorm_bindings::Class::UniqueViolation => UniqueViolationError::new_err(message.clone()),
        pgorm_bindings::Class::ForeignKeyViolation => {
            ForeignKeyViolationError::new_err(message.clone())
        }
        pgorm_bindings::Class::CheckViolation => CheckViolationError::new_err(message.clone()),
        pgorm_bindings::Class::NotNullViolation => NotNullViolationError::new_err(message.clone()),
        pgorm_bindings::Class::ExclusionViolation => {
            ExclusionViolationError::new_err(message.clone())
        }
        _ => DatabaseError::new_err(message.clone()),
    };
    decorated(exception, |value| {
        value.setattr("sqlstate", diagnostics.sqlstate)?;
        value.setattr("message", message)?;
        value.setattr("severity", diagnostics.severity)?;
        for (field, content) in [
            ("detail", diagnostics.detail),
            ("hint", diagnostics.hint),
            ("schema", diagnostics.schema),
            ("table", diagnostics.table),
            ("column", diagnostics.column),
            ("datatype", diagnostics.datatype),
            ("constraint", diagnostics.constraint),
        ] {
            value.setattr(field, content)?;
        }
        // [spec:pgorm:req:bindings.transaction-retry]
        if diagnostics.retryable {
            value.setattr("is_retryable", true)?;
        }
        Ok(())
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:pgorm:req:bindings.rollback-failed/test]    pgorm's own RollbackFailed raises both
    // errors, each as the class its own failure names
    #[test]
    fn a_failed_rollback_raises_both_errors() {
        Python::initialize();
        Python::attach(|py| {
            let failed = database_error(
                pgorm::Error::RollbackFailed {
                    cause: Box::new(pgorm::Error::Custom("aborted".into())),
                    rollback: Box::new(pgorm::Error::Custom("connection lost".into())),
                },
                &Redactions::default(),
            );
            assert!(failed.is_instance_of::<RollbackFailedError>(py));
            let value = failed.value(py).as_any();
            for field in ["cause", "rollback"] {
                let carried = value.getattr(field).ok();
                assert!(carried.is_some_and(|error| error.is_instance_of::<ConstructionError>()));
            }
        });
    }

    // [spec:pgorm:req:bindings.errors+6/test]    a failure the binding does not represent is an
    // InternalError, and an application hook's refusal the ConstructionError it is in both
    // bindings
    #[test]
    fn failures_raise_their_own_class() {
        Python::initialize();
        Python::attach(|py| {
            assert!(raised(Failure::Internal("x".into())).is_instance_of::<InternalError>(py));
            let refusal = database_error(
                pgorm::Error::Custom("before_save refused".into()),
                &Redactions::default(),
            );
            assert!(refusal.is_instance_of::<ConstructionError>(py));
        });
    }
}
