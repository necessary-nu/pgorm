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
use pgorm::{
    CreatedRangeCodec,
    pgorm_query::{ArrayType, Value},
};

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

/// A PostgreSQL type named by identifier: an enum, or a range type a schema
/// created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TypeName {
    pub(crate) name: String,
    pub(crate) schema: Option<String>,
}

/// A range or multirange type a schema created: its name, the value kind of
/// its subtype, and which of the two it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CreatedKind {
    pub(crate) name: TypeName,
    pub(crate) subtype: ArrayType,
    pub(crate) multirange: bool,
}

impl CreatedKind {
    /// pgorm's codec of this kind's values, `None` for a subtype no
    /// `RangeSubtype` is, which no kind the binding makes has.
    pub(crate) fn codec(&self) -> Option<CreatedRangeCodec> {
        CreatedRangeCodec::new(&self.subtype, self.multirange)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Tag {
    /// A kind with no type name: one of pgorm's `Value` variants.
    Scalar(ArrayType),
    Enum(TypeName),
    Created(CreatedKind),
    Array(Box<Tag>),
}

/// A value and its kind. An enum label and a created range's text form are
/// `Value::String`, as pgorm's own `ActiveEnum` and
/// `CreatedRange` types convert into.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Tagged {
    pub(crate) datum: Value,
    pub(crate) tag: Tag,
}

impl neon::types::Finalize for Tagged {}

/// The kind a pgorm value declares itself.
pub(crate) fn value_tag(value: &Value) -> Tag {
    let kind = Tag::Scalar(value.array_type());
    if value.is_array() {
        Tag::Array(Box::new(kind))
    } else {
        kind
    }
}

impl Tag {
    /// The kind's name: a scalar's own, or `enum`, `created_range`,
    /// `created_multirange` or `array`.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Scalar(kind) => kind.kind_name(),
            Self::Enum(_) => "enum",
            Self::Created(kind) if kind.multirange => "created_multirange",
            Self::Created(_) => "created_range",
            Self::Array(_) => "array",
        }
    }
}

impl Tagged {
    /// SQL NULL of a scalar kind.
    pub(crate) fn null(tag: Tag) -> Self {
        let datum = match &tag {
            Tag::Scalar(kind) => kind.null(),
            Tag::Enum(_) | Tag::Created(_) => Value::String(None),
            Tag::Array(element) => match element.as_ref() {
                Tag::Scalar(kind) => Value::Array(kind.clone(), None),
                _ => Value::Array(ArrayType::String, None),
            },
        };
        Self { datum, tag }
    }

    /// A pgorm value, tagged with the kind it declares itself.
    pub(crate) fn value(value: Value) -> Self {
        Self {
            tag: value_tag(&value),
            datum: value,
        }
    }

    pub(crate) fn is_null(&self) -> bool {
        self.datum.is_null()
    }
}
