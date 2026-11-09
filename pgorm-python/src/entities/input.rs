//! A registered column's values as Python gives and reads them, through the
//! kind pgorm-bindings says the column's values are: `None` for a column of
//! a type no kind names, which takes an explicit tagged `Value`.

use pgorm::pgorm_query::Value;
use pgorm_bindings::entity::ColumnInfo;
use pyo3::{prelude::*, types::PyString};

use crate::{
    errors::{ConstructionError, Raise},
    values::{PyValue, Tag, TagPy},
};

/// The kind as `pgorm.Value` takes it, refused for a column of no kind.
fn kind<'py>(tag: Option<&Tag>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
    match tag {
        Some(Tag::Scalar(kind)) => Ok(PyString::new(py, kind.kind_name()).into_any()),
        Some(tag @ (Tag::Enum(_) | Tag::Created(_))) => Ok(tag.to_python(py)?.into_bound(py)),
        _ => Err(ConstructionError::new_err(
            "this column requires an explicit tagged Value",
        )),
    }
}

/// `values`, a list, as one array of the column's kind, or `None` for a
/// column of an array or of no kind: the array a quantified comparison with
/// the column ranges over, a created range's the array of their texts that
/// the column's array cast reads.
pub(crate) fn array_of(column: &ColumnInfo, values: &Bound<'_, PyAny>) -> PyResult<Option<Value>> {
    match &column.tag {
        Some(Tag::Array(_)) | None => Ok(None),
        tag => {
            let py = values.py();
            let class = py.get_type::<PyValue>();
            let array: PyValue = class
                .call_method1("array", (kind(tag.as_ref(), py)?, values))?
                .extract()?;
            Ok(Some(array.inner))
        }
    }
}

/// `value` as a value of `column`: a `pgorm.Value` is held to the column's
/// enum or created range type, and a plain value converted to the column's
/// kind.
pub(crate) fn coerce(column: &ColumnInfo, value: &Bound<'_, PyAny>) -> PyResult<PyValue> {
    let tag = column.tag.as_ref();
    if let Ok(value) = value.extract::<PyRef<'_, PyValue>>() {
        let element = match tag {
            Some(Tag::Array(member)) => match member.as_ref() {
                Tag::Created(kind) => Some(kind),
                _ => None,
            },
            _ => None,
        };
        if value.created_element() != element {
            return Err(ConstructionError::new_err(
                "created range array Value belongs to a different declared column type",
            ));
        }
        if let Some(kind) = value.created()
            && !matches!(tag, Some(Tag::Created(expected)) if expected == kind)
        {
            return Err(ConstructionError::new_err(
                "created range Value belongs to a different declared column type",
            ));
        }
        if let Some(cast) = value.enum_cast() {
            let (expected, array) = match tag {
                Some(Tag::Enum(name)) => (Some(name), false),
                Some(Tag::Array(member)) => (
                    match member.as_ref() {
                        Tag::Enum(name) => Some(name),
                        _ => None,
                    },
                    true,
                ),
                _ => (None, false),
            };
            if !expected.is_some_and(|name| {
                cast.name.to_string() == name.name
                    && cast.schema.as_ref().map(|s| s.to_string()) == name.schema
                    && cast.array == array
            }) {
                return Err(ConstructionError::new_err(
                    "enum Value belongs to a different declared column type",
                ));
            }
        }
        return Ok(value.clone());
    }
    let py = value.py();
    let class = py.get_type::<PyValue>();
    let result = match tag {
        Some(Tag::Array(member)) => {
            class.call_method1("array", (kind(Some(member), py)?, value))?
        }
        _ => class.call1((value, kind(tag, py)?))?,
    };
    Ok(result.extract::<PyValue>()?)
}

/// `value`, read from `column`, as the `pgorm.Value` of the kind it reads
/// as, `ColumnInfo::tag_of`.
pub(crate) fn tagged(column: &ColumnInfo, value: Value) -> PyResult<PyValue> {
    let tag = column.tag_of(&value).py()?;
    Ok(PyValue::with_tag(value, tag))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use pgorm::pgorm_query::{ArrayType, ColumnType, Name, Range, RangeType};

    use super::*;
    use crate::values::PyRange;

    fn column(ty: &ColumnType) -> ColumnInfo {
        ColumnInfo {
            name: "c".to_owned(),
            json_key: "c".to_owned(),
            sql_type: format!("{ty:?}"),
            rust_type: None,
            nullable: true,
            primary_key: false,
            tag: Tag::of_column(ty),
            labels: None,
        }
    }

    // [spec:pgorm:req:bindings.entities+6/test]    a registered entity's range field takes a
    // `pgorm.Range` as its value, and an array of them as an array of the range kind
    #[test]
    fn range_columns_take_a_range() -> PyResult<()> {
        Python::initialize();
        Python::attach(|py| {
            let range = Py::new(
                py,
                PyRange::bounds(
                    Some(1i32.into_pyobject(py)?.into_any().unbind()),
                    true,
                    None,
                    false,
                ),
            )?;
            let value = coerce(
                &column(&ColumnType::Range(RangeType::Int4)),
                range.bind(py).as_any(),
            )?;
            assert_eq!(value.rust_value(), &Value::from(Range::from(1i32..)));
            let spans = pyo3::types::PyList::new(py, [range])?;
            let array = coerce(
                &column(&ColumnType::Array(Arc::new(ColumnType::Range(
                    RangeType::Int4,
                )))),
                spans.as_any(),
            )?;
            assert!(matches!(
                array.rust_value(),
                Value::Array(ArrayType::Range(RangeType::Int4), Some(_))
            ));
            Ok(())
        })
    }

    // [spec:pgorm:req:bindings.entities+6/test]    a registered entity's created range field
    // takes a `pgorm.Range` and reads back tagged with the type; a value of another created
    // type is refused
    #[test]
    fn created_range_columns_read_back_tagged() -> PyResult<()> {
        let created = column(&ColumnType::CreatedRange {
            name: Name::runtime("floatrange"),
            schema: Some(Name::runtime("measure")),
            subtype: Arc::new(ColumnType::Double),
        });
        let multirange = column(&ColumnType::CreatedMultirange {
            name: Name::runtime("slot_multirange"),
            schema: None,
            subtype: Arc::new(ColumnType::Integer),
        });
        Python::initialize();
        Python::attach(|py| {
            let range = Py::new(
                py,
                PyRange::bounds(
                    Some(1.5f64.into_pyobject(py)?.into_any().unbind()),
                    true,
                    None,
                    false,
                ),
            )?;
            let value = coerce(&created, range.bind(py).as_any())?;
            assert_eq!(
                value.rust_value(),
                &Value::String(Some(Box::new("[1.5,)".into())))
            );
            assert_eq!(&tagged(&created, value.rust_value().clone())?, &value);
            let other = coerce(&multirange, PyString::new(py, "{[1,2)}").as_any())?;
            assert!(coerce(&created, Py::new(py, other)?.bind(py).as_any()).is_err());
            Ok(())
        })
    }

    // [spec:pgorm:req:bindings.schema-domains+2/test]    a registered entity's domain field binds
    // as a column of the domain's base would
    #[test]
    fn domain_columns_bind_as_their_base() -> PyResult<()> {
        let quantity = column(&ColumnType::Domain {
            name: Name::runtime("quantity"),
            schema: Some(Name::runtime("billing")),
            base: Arc::new(ColumnType::Integer),
        });
        Python::initialize();
        Python::attach(|py| {
            let value = coerce(&quantity, 7i64.into_pyobject(py)?.into_any().as_any())?;
            assert_eq!(value.rust_value(), &Value::Int(Some(7)));
            Ok(())
        })
    }
}
