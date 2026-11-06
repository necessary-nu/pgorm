//! A panic in native work becomes the `InternalError` the shared rules name,
//! rather than pyo3-async-runtimes' own `RustPanic`.

use std::{any::Any, future::Future, panic::AssertUnwindSafe};

use futures_util::FutureExt;
use pyo3::{prelude::*, types::PyAny};

use crate::errors::InternalError;

/// pyo3-async-runtimes' `future_into_py`, its future's panic resolving the
/// awaitable with an `InternalError` naming the panic's message.
///
/// The future runs on the runtime, where nothing of the caller's is held
/// across the unwind but what the future owns, which the unwind drops, so
/// asserting it unwind-safe observes no broken invariant.
// [spec:pgorm:req:python.runtime+2]
pub(crate) fn future_into_py<F, T>(py: Python<'_>, fut: F) -> PyResult<Bound<'_, PyAny>>
where
    F: Future<Output = PyResult<T>> + Send + 'static,
    T: for<'py> IntoPyObject<'py> + Send + 'static,
{
    pyo3_async_runtimes::tokio::future_into_py(
        py,
        AssertUnwindSafe(fut).catch_unwind().map(|outcome| {
            outcome.unwrap_or_else(|payload| {
                Err(InternalError::new_err(format!(
                    "pgorm panicked: {}",
                    panic_message(payload.as_ref())
                )))
            })
        }),
    )
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic with no message")
}

/// `_probe_panic()`, in debug builds alone: an awaitable whose work panics on
/// the runtime, for the suite to prove the panic arrives as an
/// `InternalError`, as pgorm-napi's `probePanic` proves its own.
#[cfg(debug_assertions)]
#[pyfunction(name = "_probe_panic")]
fn probe_panic(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    future_into_py::<_, ()>(py, async { panic!("the probe's deliberate panic") })
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    #[cfg(debug_assertions)]
    module.add_function(wrap_pyfunction!(probe_panic, module)?)?;
    #[cfg(not(debug_assertions))]
    let _ = module;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_messages_read_from_both_payload_types() {
        let text: Box<dyn Any + Send> = Box::new("static");
        let owned: Box<dyn Any + Send> = Box::new(String::from("owned"));
        let other: Box<dyn Any + Send> = Box::new(7_u8);
        assert_eq!(panic_message(text.as_ref()), "static");
        assert_eq!(panic_message(owned.as_ref()), "owned");
        assert_eq!(panic_message(other.as_ref()), "a panic with no message");
    }
}
