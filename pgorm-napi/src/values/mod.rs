//! pgorm's values as the binding holds them: a `pgorm_query::Value` with the
//! identity plain JavaScript cannot carry — an enum's type, a created range's
//! type and subtype, an array's element kind.

pub(crate) mod json;
pub(crate) mod read;
mod value;
pub(crate) mod write;

use neon::prelude::{
    Context, FunctionContext, JsArray, JsResult, ModuleContext, NeonResult, Object,
};
use pgorm::pgorm_query::{ArrayType, Value};

use crate::errors::Failure;

/// `Value`'s native half, and `valueKinds()`.
pub(crate) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    value::export(cx)?;
    cx.export_function("valueKinds", kinds)
}

/// `valueKinds()`: every kind pgorm names, as `Value` takes it: the scalar
/// kinds, then each built-in range type's range and multirange.
fn kinds(mut cx: FunctionContext) -> JsResult<JsArray> {
    let kinds = JsArray::new(&mut cx, 0);
    for (index, kind) in ArrayType::kinds().enumerate() {
        let name = cx.string(kind.kind_name());
        kinds.set(&mut cx, u32::try_from(index).unwrap_or(u32::MAX), name)?;
    }
    Ok(kinds)
}

/// The refusal of a value, or value kind, of pgorm's that this binding has no
/// JavaScript form of. pgorm's value vocabulary grows, and a kind added after
/// the binding was written is reported rather than read as another.
pub(crate) fn unrepresented() -> Failure {
    Failure::Internal("a kind of pgorm value pgorm-napi does not represent".to_owned())
}

pub(crate) use pgorm_bindings::{CreatedKind, Tag, TypeName};

/// A value and its kind. An enum label and a created range's text form are
/// `Value::String`, as pgorm's own `ActiveEnum` and
/// `CreatedRange` types convert into.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Tagged {
    pub(crate) datum: Value,
    pub(crate) tag: Tag,
}

impl neon::types::Finalize for Tagged {}

impl Tagged {
    /// SQL NULL of a kind.
    pub(crate) fn null(tag: Tag) -> Self {
        Self {
            datum: tag.null(),
            tag,
        }
    }

    /// A pgorm value, tagged with the kind it declares itself.
    pub(crate) fn value(value: Value) -> Self {
        Self {
            tag: Tag::of_value(&value),
            datum: value,
        }
    }

    pub(crate) fn is_null(&self) -> bool {
        self.datum.is_null()
    }
}
