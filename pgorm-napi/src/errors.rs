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
use pgorm_bindings::{Class, Dialect};

/// The factory `lib/index.js` registers: `(kind, message, details) => Error`.
///
/// Instance-local because a class belongs to one JavaScript realm: a worker
/// thread loading the addon registers its own.
static ERROR_FACTORY: LocalKey<Mutex<Option<Root<JsFunction>>>> = LocalKey::new();

pub(crate) use pgorm_bindings::{Failure, Redactions};

/// A failure as the error object a promise rejects with.
pub(crate) trait FailureJs {
    /// The class name `lib/index.js` maps the failure to.
    fn kind(&self) -> &'static str;
    /// The error object a promise rejects with.
    fn into_js<'cx>(self, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue>;
}

impl FailureJs for Failure {
    fn kind(&self) -> &'static str {
        match self.class() {
            Class::RollbackFailed => "SuppressedError",
            class => class.name(),
        }
    }

    fn into_js<'cx>(self, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        // [spec:pgorm:req:bindings.rollback-failed]
        if let Self::RollbackFailed {
            message,
            cause,
            rollback,
        } = self
        {
            let rollback = rollback.into_js(cx)?;
            let cause = cause.into_js(cx)?;
            let constructor: Handle<JsFunction> = cx.global("SuppressedError")?;
            return constructor
                .bind(cx)
                .arg(rollback)?
                .arg(cause)?
                .arg(message)?
                .construct();
        }
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
            // [spec:pgorm:req:bindings.transaction-retry]
            details.prop(cx, "isRetryable").set(diagnostics.retryable)?;
        }
        // [spec:pgorm:req:bindings.refusal-fields]
        let text = |cx: &mut Cx<'cx>, value: &Option<String>| -> Handle<'cx, JsValue> {
            match value {
                Some(text) => cx.string(text).upcast(),
                None => cx.null().upcast(),
            }
        };
        if let Self::Bind {
            parameter, reason, ..
        } = &self
        {
            #[allow(clippy::cast_precision_loss)]
            let parameter = cx.number(*parameter as f64);
            details.prop(cx, "parameter").set(parameter)?;
            let reason = text(cx, reason);
            details.prop(cx, "reason").set(reason)?;
        }
        if let Self::Unreadable {
            reading, reason, ..
        } = &self
        {
            let reading = cx.string(reading);
            details.prop(cx, "reading").set(reading)?;
            let reason = text(cx, reason);
            details.prop(cx, "reason").set(reason)?;
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

/// Classify a pgorm error as `bindings.errors` does for JavaScript.
pub(crate) fn failure(error: &pgorm::Error, secrets: &Redactions) -> Failure {
    pgorm_bindings::classify(error, secrets, Dialect::JavaScript)
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

    // [spec:pgorm:req:bindings.rollback-failed/test]    a failed rollback rejects as JavaScript's
    // SuppressedError, and every other failure as the class both bindings name
    #[test]
    fn a_failure_names_the_javascript_class_it_becomes() {
        let failed = Failure::rollback_failed(
            Failure::Construction("aborted".into()),
            Failure::Connection("connection lost".into()),
        );
        assert_eq!(failed.kind(), "SuppressedError");
        assert_eq!(Failure::Cancelled.kind(), "CancelledError");
        assert_eq!(Failure::Decode("x".into()).kind(), "DecodeError");
    }
}
