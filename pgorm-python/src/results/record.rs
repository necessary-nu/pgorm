use std::collections::{HashMap, HashSet};

use pyo3::{exceptions::PyKeyError, prelude::*, types::PyTuple};
use tokio_postgres::Row;

use crate::{
    errors::DecodeError,
    values::{PyTypeName, PyValue},
};

/// Owned PostgreSQL output identity, including source identity when available.
#[derive(Clone, Debug)]
#[pyclass(name = "Field", module = "pgorm", frozen, get_all, from_py_object)]
pub struct PyField {
    pub name: String,
    pub index: usize,
    pub type_name: PyTypeName,
    pub type_oid: u32,
    pub table_oid: Option<u32>,
    pub column_id: Option<i16>,
}

// [spec:pgorm:req:bindings.rows]
// [spec:pgorm:req:python.records]
/// A detached row whose unique output names index exact, tagged Rust values.
#[derive(Clone, Debug)]
#[pyclass(name = "Record", module = "pgorm", frozen, from_py_object)]
pub struct PyRecord {
    fields: Vec<PyField>,
    values: Vec<PyValue>,
    names: HashMap<String, usize>,
}

impl PyRecord {
    pub(crate) fn decode(row: &Row) -> PyResult<Self> {
        let mut seen = HashSet::new();
        let mut fields = Vec::with_capacity(row.len());
        let mut values = Vec::with_capacity(row.len());
        for (index, column) in row.columns().iter().enumerate() {
            if !seen.insert(column.name()) {
                return Err(DecodeError::new_err(
                    "duplicate result field name; give each projection a unique alias",
                ));
            }
            fields.push(PyField {
                name: column.name().to_owned(),
                index,
                type_name: PyTypeName {
                    name: column.type_().name().to_owned(),
                    schema: Some(column.type_().schema().to_owned()),
                },
                type_oid: column.type_().oid(),
                table_oid: column.table_oid(),
                column_id: column.column_id(),
            });
            let value = super::decode::value(super::decode::Field::Row(row, index))?;
            // A successful terminal guarantees representability in Python now,
            // including temporal bounds and f32 NaN payloads, not at a later getter.
            Python::attach(|py| value.to_python(py))?;
            values.push(value);
        }
        let names = fields
            .iter()
            .map(|field| (field.name.clone(), field.index))
            .collect();
        Ok(Self {
            fields,
            values,
            names,
        })
    }

    /// Read the column `name`, which holds an enum's label as its text, as
    /// a value of the enum `enum_type`, or an array of it.
    pub(crate) fn read_as_enum(&mut self, name: &str, enum_type: PyTypeName, array: bool) {
        if let Some(index) = self.names.get(name) {
            let value = self.values[*index].rust_value().clone();
            if matches!(
                value,
                pgorm::pgorm_query::Value::String(_) | pgorm::pgorm_query::Value::Array(..)
            ) {
                self.values[*index] = PyValue::from_enum(value, enum_type, array);
            }
        }
    }

    /// The value of the column `name`, if the record has one.
    pub(crate) fn value(&self, name: &str) -> Option<&PyValue> {
        self.names.get(name).map(|index| &self.values[*index])
    }

    fn position(&self, name: &str) -> PyResult<usize> {
        self.names
            .get(name)
            .copied()
            .ok_or_else(|| PyKeyError::new_err(name.to_owned()))
    }
}

#[pymethods]
impl PyRecord {
    fn __len__(&self) -> usize {
        self.values.len()
    }

    fn __contains__(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    fn __getitem__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        self.values[self.position(name)?].to_python(py)
    }

    #[pyo3(signature = (name, default=None))]
    fn get(&self, py: Python<'_>, name: &str, default: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        match self.names.get(name) {
            Some(index) => self.values[*index].to_python(py),
            None => Ok(default.unwrap_or_else(|| py.None())),
        }
    }

    fn keys<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.fields.iter().map(|field| field.name.as_str()))
    }

    fn values<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(
            py,
            self.values
                .iter()
                .map(|v| v.to_python(py))
                .collect::<PyResult<Vec<_>>>()?,
        )
    }

    fn items<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let items = self
            .fields
            .iter()
            .zip(&self.values)
            .map(|(field, value)| Ok((field.name.clone(), value.to_python(py)?)))
            .collect::<PyResult<Vec<_>>>()?;
        PyTuple::new(py, items)
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.keys(py)?.call_method0("__iter__")
    }

    fn tagged(&self, name: &str) -> PyResult<PyValue> {
        Ok(self.values[self.position(name)?].clone())
    }

    #[getter]
    fn fields<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.fields.iter().cloned())
    }

    fn __repr__(&self) -> String {
        format!("Record(fields={})", self.fields.len())
    }
}
