//! The rows of a registered `Select<E>`'s and `SelectGraph<E, S>`'s reads as
//! Python receives them, and the decode their stream runs on each row as it
//! arrives.

use std::{fmt, num::NonZeroU64, sync::Arc};

use pgorm::QueryResult;
use pgorm_bindings::entity::{Decode, Row};
use pyo3::{prelude::*, types::PyTuple};

use super::PyEntityModel;
use crate::errors::DecodeError;

/// A row as Python receives it: a read of one source's model alone, `None`
/// for an absent optional one, and otherwise a tuple of each source's, each
/// model held to its columns' Python values.
pub(crate) fn row_object(py: Python<'_>, row: Row) -> PyResult<Py<PyAny>> {
    let row: Vec<Option<PyEntityModel>> = row
        .into_iter()
        .map(|model| model.map(|inner| PyEntityModel { inner }))
        .collect();
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
// [spec:pgorm:req:bindings.streams+1]
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

/// A page size as pgorm's paginator takes one, refused below 1.
pub(crate) fn page_size(size: u64) -> PyResult<NonZeroU64> {
    NonZeroU64::new(size)
        .ok_or_else(|| crate::errors::ConstructionError::new_err("a page size is at least 1"))
}
