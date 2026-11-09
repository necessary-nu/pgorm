//! The binary COPY format's two sides, for a program that moves the bytes
//! itself: `CopyRowEncoder` and `CopyRowDecoder`, pgorm's own under Python
//! classes.

use std::sync::Mutex;

use bytes::BytesMut;
use pgorm::{CopyRowDecoder, CopyRowEncoder};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyTuple},
};

use super::{
    columns::PyCopyColumn,
    native::{refusal, row_values},
};
use crate::{
    errors::{ConstructionError, DecodeError, InternalError},
    results::{Field, value},
};

fn columns(columns: Vec<PyRef<'_, PyCopyColumn>>) -> Vec<pgorm::CopyColumn> {
    columns.iter().map(|column| column.inner.clone()).collect()
}

/// The binary format's writing side over `columns`: the header, each row,
/// each value read as a parameter is and checked against its column's type,
/// and the trailer, each as bytes.
// [spec:pgorm:req:bindings.copy-rows+2]
#[pyclass(name = "CopyRowEncoder", module = "pgorm", frozen)]
pub(crate) struct PyCopyRowEncoder {
    encoder: CopyRowEncoder,
}

impl std::fmt::Debug for PyCopyRowEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCopyRowEncoder").finish_non_exhaustive()
    }
}

#[pymethods]
impl PyCopyRowEncoder {
    #[new]
    fn new(columns: Vec<PyRef<'_, PyCopyColumn>>) -> Self {
        Self {
            encoder: CopyRowEncoder::new(self::columns(columns)),
        }
    }

    #[getter]
    fn columns(&self) -> Vec<PyCopyColumn> {
        self.encoder
            .columns()
            .iter()
            .map(|column| PyCopyColumn {
                inner: column.clone(),
            })
            .collect()
    }

    #[staticmethod]
    fn header(py: Python<'_>) -> Bound<'_, PyBytes> {
        let mut out = BytesMut::new();
        CopyRowEncoder::header(&mut out);
        PyBytes::new(py, &out)
    }

    #[staticmethod]
    fn trailer(py: Python<'_>) -> Bound<'_, PyBytes> {
        let mut out = BytesMut::new();
        CopyRowEncoder::trailer(&mut out);
        PyBytes::new(py, &out)
    }

    /// One row, a sequence of its columns' values. A row of another width,
    /// or a value its column's type does not take, is a ConstructionError
    /// naming the column.
    fn encode<'py>(
        &self,
        py: Python<'py>,
        row: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let values = row_values(row, self.encoder.columns())?;
        let mut out = BytesMut::new();
        self.encoder
            .encode(&values, &mut out)
            .map_err(|error| ConstructionError::new_err(refusal(error)))?;
        Ok(PyBytes::new(py, &out))
    }
}

/// The binary format's reading side over `columns`: data pushed in chunks
/// split anywhere, each row given once all of it has arrived, each field
/// decoded by its column's type as `copy_out_rows` decodes it.
// [spec:pgorm:req:bindings.copy-rows+2]
#[pyclass(name = "CopyRowDecoder", module = "pgorm", frozen)]
pub(crate) struct PyCopyRowDecoder {
    decoder: Mutex<CopyRowDecoder>,
}

impl std::fmt::Debug for PyCopyRowDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCopyRowDecoder").finish_non_exhaustive()
    }
}

impl PyCopyRowDecoder {
    fn lock(&self) -> PyResult<std::sync::MutexGuard<'_, CopyRowDecoder>> {
        self.decoder
            .lock()
            .map_err(|_| InternalError::new_err("COPY decoder lock failed"))
    }
}

#[pymethods]
impl PyCopyRowDecoder {
    #[new]
    fn new(columns: Vec<PyRef<'_, PyCopyColumn>>) -> Self {
        Self {
            decoder: Mutex::new(CopyRowDecoder::new(self::columns(columns))),
        }
    }

    #[getter]
    fn columns(&self) -> PyResult<Vec<PyCopyColumn>> {
        Ok(self
            .lock()?
            .columns()
            .iter()
            .map(|column| PyCopyColumn {
                inner: column.clone(),
            })
            .collect())
    }

    fn push(&self, chunk: &[u8]) -> PyResult<()> {
        self.lock()?.push(chunk);
        Ok(())
    }

    /// The next row whole in the data pushed so far, a tuple of values, or
    /// None until more arrives. A field that does not decode is a
    /// DecodeError naming its column.
    fn next_row<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyTuple>>> {
        let mut decoder = self.lock()?;
        let Some(fields) = decoder
            .next_fields()
            .map_err(|error| DecodeError::new_err(refusal(error)))?
        else {
            return Ok(None);
        };
        let values = fields
            .iter()
            .zip(decoder.columns())
            .enumerate()
            .map(|(index, (raw, column))| {
                value(Field::Copy {
                    column,
                    raw: raw.as_deref(),
                    index,
                })?
                .to_python(py)
            })
            .collect::<PyResult<Vec<_>>>()?;
        PyTuple::new(py, values).map(Some)
    }

    /// Check the data ended where a row did, with the format's trailer.
    fn finish(&self) -> PyResult<()> {
        self.lock()?
            .finish()
            .map_err(|error| DecodeError::new_err(refusal(error)))
    }
}
