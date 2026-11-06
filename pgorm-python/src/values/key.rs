//! A kind spelled as one string, as a model's and a registered entity's
//! `describe()` report it: `i64`, `text[]`, `enum "app"."mood"`,
//! `range "app"."floatrange" of f64`.

use pyo3::prelude::*;

use super::{PyTypeName, PyValue, types::Tag};

/// A type's name double-quoted, its own quotes doubled, behind its quoted
/// schema when it has one.
fn quoted(name: &PyTypeName) -> String {
    let part = |part: &str| format!("\"{}\"", part.replace('"', "\"\""));
    match &name.schema {
        Some(schema) => format!("{}.{}", part(schema), part(&name.name)),
        None => part(&name.name),
    }
}

impl Tag {
    // [spec:pgorm:req:bindings.models+6]
    pub(crate) fn key(&self) -> String {
        match self {
            Self::Scalar(kind) => kind.kind_name().to_owned(),
            Self::Enum(name) => format!("enum {}", quoted(name)),
            Self::Created(kind) => format!(
                "{} {} of {}",
                if kind.multirange {
                    "multirange"
                } else {
                    "range"
                },
                quoted(&kind.name),
                kind.subtype.kind_name()
            ),
            Self::Array(element) => format!("{}[]", element.key()),
        }
    }
}

#[pymethods]
impl PyValue {
    /// The value's kind as a description spells it.
    fn _kind_key(&self) -> String {
        self.tag.key()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::CreatedKind;
    use pgorm::pgorm_query::{ArrayType, RangeType};

    fn name(name: &str, schema: Option<&str>) -> PyTypeName {
        PyTypeName {
            name: name.to_owned(),
            schema: schema.map(str::to_owned),
        }
    }

    // [spec:pgorm:req:bindings.models+6/test]    a kind is spelled as a decoded column's is
    // compared: a scalar's name, an enum's quoted type, a created range's type and subtype,
    // and an array's element followed by `[]`
    #[test]
    fn kinds_are_spelled_as_results_compare_them() {
        assert_eq!(Tag::Scalar(ArrayType::BigInt).key(), "i64");
        assert_eq!(
            Tag::Scalar(ArrayType::Range(RangeType::Int4)).key(),
            "int4range"
        );
        assert_eq!(
            Tag::Array(Box::new(Tag::Scalar(ArrayType::String))).key(),
            "text[]"
        );
        assert_eq!(
            Tag::Enum(name("mood", Some("app"))).key(),
            "enum \"app\".\"mood\""
        );
        assert_eq!(
            Tag::Array(Box::new(Tag::Enum(name("a\"b", None)))).key(),
            "enum \"a\"\"b\"[]"
        );
        let created = |multirange| CreatedKind {
            name: name("floatrange", Some("app")),
            subtype: ArrayType::Double,
            multirange,
        };
        assert_eq!(
            Tag::Created(created(false)).key(),
            "range \"app\".\"floatrange\" of f64"
        );
        assert_eq!(
            Tag::Created(created(true)).key(),
            "multirange \"app\".\"floatrange\" of f64"
        );
    }
}
