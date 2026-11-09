//! Each row of a disjoint read, judged by pgorm's shape and read back by the
//! codec.

use pgorm::QueryResult;
use pgorm_bindings::entity::{DisjointCore, RowCodec};
use pyo3::{prelude::*, types::PyTuple};
use tokio_postgres::Row;

use crate::{entities::PyEntityModel, errors::Raise, results::PyRecord, values::Tag};

/// One decoded row: a model's `Record` beside the index of its kind and the
/// decoded source its kind's row is under, which Python views by field, or a
/// registration's base model, kind and kind model.
pub(crate) enum Item {
    Record {
        record: PyRecord,
        kind: usize,
        source: Option<usize>,
    },
    Entity {
        base: PyEntityModel,
        kind: String,
        row: Option<PyEntityModel>,
    },
}

impl Item {
    /// The row as Python reads it: `(Record, kind index, source or None)` or
    /// `(EntityModel, kind, EntityModel or None)`, each registered model held
    /// to its columns' Python values as an entity read's is.
    pub(crate) fn into_python(self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self {
            Self::Record {
                record,
                kind,
                source,
            } => Ok(PyTuple::new(
                py,
                [
                    Py::new(py, record)?.into_any(),
                    kind.into_pyobject(py)?.into_any().unbind(),
                    source.into_pyobject(py)?.into_any().unbind(),
                ],
            )?
            .into_any()
            .unbind()),
            Self::Entity { base, kind, row } => {
                base.validate(py)?;
                if let Some(row) = &row {
                    row.validate(py)?;
                }
                let row = match row {
                    Some(row) => Py::new(py, row)?.into_any(),
                    None => py.None(),
                };
                Ok(PyTuple::new(
                    py,
                    [
                        Py::new(py, base)?.into_any(),
                        kind.into_pyobject(py)?.into_any().unbind(),
                        row,
                    ],
                )?
                .into_any()
                .unbind())
            }
        }
    }
}

/// Judge `row` by its kind and read it back.
// [spec:pgorm:req:bindings.disjoint+2]
pub(crate) fn decode(core: &DisjointCore, row: Row) -> PyResult<Item> {
    if matches!(core.codec, RowCodec::Entities { .. }) {
        let row = core.entity_row(&QueryResult::from(row)).py()?;
        return Ok(Item::Entity {
            base: PyEntityModel { inner: row.base },
            kind: row.kind,
            row: row.row.map(|inner| PyEntityModel { inner }),
        });
    }
    let mut record = PyRecord::decode(&row)?;
    for (name, tag) in core.enum_columns() {
        let (array, tag) = match tag {
            Tag::Array(element) => (true, *element),
            tag => (false, tag),
        };
        if let Tag::Enum(name_of_type) = tag {
            record.read_as_enum(&name, name_of_type, array);
        }
    }
    let judged = core
        .judge(&QueryResult::from(row), |name| {
            record
                .value(name)
                .map(|value| value.rust_value().to_string())
        })
        .py()?;
    Ok(Item::Record {
        record,
        kind: judged.kind,
        source: judged.source,
    })
}
