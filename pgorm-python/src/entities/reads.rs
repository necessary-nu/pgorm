//! The reads a registered `Select<E>` and `SelectGraph<E, S>` have beside
//! `all` and `one_opt`: their pages and their keyset cursor, pgorm's erased
//! paginator and cursor, and their rows as a stream, each decoded by the
//! read's own selector as it arrives.

use std::{fmt, num::NonZeroU64, sync::Arc};

use pgorm::{ErasedCursor, ErasedPages, Error, QueryResult};
use pyo3::{prelude::*, types::PyTuple};

use super::{PyEntityModel, metadata::EntityInfo};
use crate::{errors::DecodeError, graphs::backend::Row, metrics::Metrics};

/// A read's pages, pgorm's erased paginator over the read's rows.
pub(crate) type Pages = dyn ErasedPages<Row, Metrics>;

/// A column a cursor's boundary value converts through: its entity, and its
/// name there.
#[derive(Clone, Debug)]
pub(crate) struct KeyColumn {
    pub(crate) entity: Arc<EntityInfo>,
    pub(crate) column: String,
}

/// A registered read's keyset cursor: pgorm's erased cursor, its order
/// columns, and the whole key a `_with` boundary names.
#[derive(Debug)]
pub(crate) struct Keyset {
    pub(crate) cursor: Arc<dyn ErasedCursor<Row, Metrics>>,
    pub(crate) order: Vec<KeyColumn>,
    pub(crate) key: Vec<KeyColumn>,
}

/// The decode of one row of a registered read's result into its sources'
/// models, as its selector's `from_raw_query_result` reads it.
pub(crate) type Decode = Arc<dyn Fn(QueryResult) -> Result<Row, Error> + Send + Sync>;

/// A row as Python receives it: a read of one source's model alone, `None`
/// for an absent optional one, and otherwise a tuple of each source's.
pub(crate) fn row_object(py: Python<'_>, row: Row) -> PyResult<Py<PyAny>> {
    for model in row.iter().flatten() {
        model.validate(py)?;
    }
    if row.len() == 1 {
        return Ok(match row.into_iter().next().flatten() {
            Some(model) => Py::new(py, model)?.into_any(),
            None => py.None(),
        });
    }
    Ok(PyTuple::new(py, row)?.into_any().unbind())
}

/// The decode a registered read's stream runs on each row, handed to a
/// connection's `stream` beside the read's statement.
// [spec:pgorm:req:bindings.streams]
#[pyclass(name = "_RowDecoder", module = "pgorm._native", frozen)]
pub(crate) struct NativeRowDecoder {
    pub(crate) decode: Decode,
}

impl fmt::Debug for NativeRowDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeRowDecoder").finish_non_exhaustive()
    }
}

impl NativeRowDecoder {
    pub(crate) fn row_decode(&self) -> crate::results::RowDecode {
        let decode = self.decode.clone();
        Arc::new(move |row| {
            let row = decode(QueryResult::from(row))
                .map_err(|error| DecodeError::new_err(error.to_string()))?;
            Python::attach(|py| row_object(py, row))
        })
    }
}

/// The model a registered entity's row decodes as, in a read of that entity
/// alone.
pub(crate) fn one_source(model: PyEntityModel) -> Row {
    vec![Some(model)]
}

/// A page size as pgorm's paginator takes one, refused below 1.
pub(crate) fn page_size(size: u64) -> PyResult<NonZeroU64> {
    NonZeroU64::new(size)
        .ok_or_else(|| crate::errors::ConstructionError::new_err("a page size is at least 1"))
}
