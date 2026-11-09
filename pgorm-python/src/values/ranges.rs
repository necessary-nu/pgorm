//! Ranges and multiranges between `pgorm.Range` / `pgorm.Multirange` and the
//! Rust value model.
//!
//! A range is tagged with its built-in range type, named as PostgreSQL names
//! it (`int4range`), and each bound converts exactly as a scalar of the
//! subtype's kind does.

use std::ops;

use pgorm::pgorm_query::{ArrayType, Multirange, Range, RangeType, Value};
use pyo3::prelude::*;

use super::{
    classes::{PyMultirange, PyRange},
    convert,
};
use crate::errors::ConstructionError;

/// The kinds of the six range types and their multiranges, named as
/// PostgreSQL names them.
pub(crate) fn kind_names() -> impl Iterator<Item = &'static str> {
    ArrayType::kinds()
        .filter(|kind| matches!(kind, ArrayType::Range(_) | ArrayType::Multirange(_)))
        .map(|kind| kind.kind_name())
}

// [spec:pgorm:req:bindings.values]
pub(super) fn multirange_from_python(data: &Bound<'_, PyAny>, range: RangeType) -> PyResult<Value> {
    let multirange = data
        .cast_exact::<PyMultirange>()
        .map_err(|_| ConstructionError::new_err("expected pgorm.Multirange"))?;
    let element = range.element_type();
    let ranges = multirange
        .get()
        .ranges
        .iter()
        .map(|item| read_bounds(item.bind(data.py()).as_any(), &element))
        .collect::<PyResult<Multirange<Value>>>()?;
    Ok(Value::Multirange(range, Some(Box::new(ranges))))
}

/// A `pgorm.Range`'s bounds as values of the scalar kind `kind`, each
/// converted with that kind's limits; `None` on a side is no bound.
// [spec:pgorm:req:bindings.values]
pub(super) fn read_bounds(data: &Bound<'_, PyAny>, kind: &ArrayType) -> PyResult<Range<Value>> {
    let data = data
        .cast_exact::<PyRange>()
        .map_err(|_| ConstructionError::new_err("expected pgorm.Range"))?;
    let py = data.py();
    let data = data.get();
    if data.empty {
        return Ok(Range::Empty);
    }
    let side = |value: &Option<Py<PyAny>>, inclusive: bool| -> PyResult<ops::Bound<Value>> {
        let Some(value) = value else {
            return Ok(ops::Bound::Unbounded);
        };
        let value = convert::from_python(value.bind(py), kind)?;
        Ok(if inclusive {
            ops::Bound::Included(value)
        } else {
            ops::Bound::Excluded(value)
        })
    };
    Ok(Range::new(
        side(&data.lower, data.lower_inc)?,
        side(&data.upper, data.upper_inc)?,
    ))
}

// [spec:pgorm:req:bindings.values]
pub(super) fn multirange_to_python(
    py: Python<'_>,
    multirange: &Multirange<Value>,
) -> PyResult<Py<PyAny>> {
    let ranges = multirange
        .iter()
        .map(|range| Py::new(py, write(py, range)?))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(Py::new(py, PyMultirange { ranges })?.into_any())
}

/// A Rust range as a `pgorm.Range`. A NULL bound is no bound, as it is to
/// PostgreSQL's range constructors and to the Rust binding.
// [spec:pgorm:req:bindings.values]
pub(super) fn write(py: Python<'_>, range: &Range<Value>) -> PyResult<PyRange> {
    let side = |bound: &ops::Bound<Value>| -> PyResult<(Option<Py<PyAny>>, bool)> {
        Ok(match bound {
            ops::Bound::Included(value) | ops::Bound::Excluded(value) if !value.is_null() => (
                Some(convert::to_python(py, value)?),
                matches!(bound, ops::Bound::Included(_)),
            ),
            _ => (None, false),
        })
    };
    Ok(match range {
        Range::Empty => PyRange::empty(),
        Range::Bounds { lower, upper } => {
            let (lower, lower_inc) = side(lower)?;
            let (upper, upper_inc) = side(upper)?;
            PyRange::bounds(lower, lower_inc, upper, upper_inc)
        }
    })
}
