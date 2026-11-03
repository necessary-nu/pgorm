//! Each row of a disjoint read, judged by pgorm's shape and read back by the
//! codec.

use pgorm::QueryResult;
use pgorm::pgorm_query::ColumnType;
use pyo3::{prelude::*, types::PyTuple};
use tokio_postgres::Row;

use super::{Codec, Core};
use crate::{entities::PyEntityModel, errors::DecodeError, results::PyRecord, values::PyTypeName};

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

impl Core {
    /// Judge `row` by its kind and read it back.
    // [spec:pgorm:req:bindings.disjoint+1]
    pub(crate) fn decode(&self, row: Row) -> PyResult<Item> {
        match &self.codec {
            Codec::Models => {
                let mut record = PyRecord::decode(&row)?;
                let shape = &self.shape;
                read_enums(shape, &mut record);
                let key = || {
                    let prefix = pgorm::source_prefix(0);
                    shape
                        .base()
                        .key()
                        .map(|column| {
                            record
                                .value(&pgorm::result_column_name(
                                    &prefix,
                                    &column.name().to_string(),
                                ))
                                .map(|value| value.rust_value().to_string())
                                .unwrap_or_default()
                        })
                        .collect()
                };
                let judged = shape
                    .judge(&QueryResult::from(row), key)
                    .map_err(|error| DecodeError::new_err(error.to_string()))?;
                Ok(Item::Record {
                    record,
                    kind: judged.kind,
                    source: judged.source,
                })
            }
            Codec::Entities { base, kinds } => {
                let shape = &self.shape;
                let result = QueryResult::from(row);
                let model = base
                    .decode(&result, &pgorm::source_prefix(0))
                    .map_err(|error| DecodeError::new_err(error.to_string()))?;
                let key = || {
                    shape
                        .base()
                        .key()
                        .map(|column| {
                            model
                                .get(&column.name().to_string())
                                .map(|value| value.to_string())
                                .unwrap_or_default()
                        })
                        .collect()
                };
                let judged = shape
                    .judge(&result, key)
                    .map_err(|error| DecodeError::new_err(error.to_string()))?;
                let row = match (judged.source, &kinds[judged.kind]) {
                    (Some(source), Some(entity)) => Some(PyEntityModel {
                        inner: entity
                            .decode(&result, &pgorm::source_prefix(source))
                            .map_err(|error| DecodeError::new_err(error.to_string()))?,
                    }),
                    _ => None,
                };
                Ok(Item::Entity {
                    base: PyEntityModel { inner: model },
                    kind: shape.kinds()[judged.kind].value().to_owned(),
                    row,
                })
            }
        }
    }
}

/// Read each enum column of every source as its enum: the read projects one
/// as its text, as pgorm's `ColumnTrait::select_as` reads an entity's, and a
/// model's field is the enum its column names.
fn read_enums(shape: &pgorm::DisjointShape, record: &mut PyRecord) {
    let tables = std::iter::once((0, shape.base()))
        .chain(shape.sources().map(|(source, _, table)| (source, table)));
    for (source, table) in tables {
        let names = shape.result_names(source).unwrap_or_default();
        for (column, result_name) in table.columns().iter().zip(&names) {
            let (column_type, array) = match column.def().get_column_type() {
                ColumnType::Array(element) => (element.as_ref(), true),
                column_type => (column_type, false),
            };
            if let ColumnType::Enum { name, schema, .. } = column_type {
                record.read_as_enum(
                    result_name,
                    PyTypeName {
                        name: name.to_string(),
                        schema: schema.as_ref().map(|schema| schema.to_string()),
                    },
                    array,
                );
            }
        }
    }
}
