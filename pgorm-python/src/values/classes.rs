//! `pgorm.Range` and `pgorm.Multirange`: PostgreSQL's range values as
//! immutable Python objects, holding Python bound values.
//!
//! They are native classes rather than Python ones so the value conversion
//! reads and builds them without importing the package that loads it.

use pyo3::{
    basic::CompareOp,
    prelude::*,
    types::{PyString, PyTuple},
};

use crate::errors::ConstructionError;

/// The empty range, or every value between two bounds.
///
/// `None` on a side is no bound there, and a side with no bound includes
/// nothing, so its bracket is always `(` or `)`.
// [spec:pgorm:req:bindings.values]
// [spec:pgorm:req:python.values+7]
#[pyclass(name = "Range", module = "pgorm", frozen, generic)]
#[derive(Debug)]
pub struct PyRange {
    pub(crate) lower: Option<Py<PyAny>>,
    pub(crate) upper: Option<Py<PyAny>>,
    pub(crate) lower_inc: bool,
    pub(crate) upper_inc: bool,
    pub(crate) empty: bool,
}

impl PyRange {
    /// The range a Rust value's bounds describe.
    pub(crate) fn bounds(
        lower: Option<Py<PyAny>>,
        lower_inc: bool,
        upper: Option<Py<PyAny>>,
        upper_inc: bool,
    ) -> Self {
        Self {
            lower_inc: lower_inc && lower.is_some(),
            upper_inc: upper_inc && upper.is_some(),
            lower,
            upper,
            empty: false,
        }
    }

    fn key<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(
            py,
            [
                self.empty.into_pyobject(py)?.to_owned().into_any(),
                self.lower
                    .as_ref()
                    .map_or_else(|| py.None(), |v| v.clone_ref(py))
                    .into_bound(py),
                self.upper
                    .as_ref()
                    .map_or_else(|| py.None(), |v| v.clone_ref(py))
                    .into_bound(py),
                self.bounds_text().into_pyobject(py)?.into_any(),
            ],
        )
    }
}

#[pymethods]
impl PyRange {
    #[new]
    #[pyo3(signature = (lower=None, upper=None, bounds=None))]
    fn new(
        lower: Option<Py<PyAny>>,
        upper: Option<Py<PyAny>>,
        bounds: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let bounds = match bounds {
            None => "[)",
            Some(bounds) => bounds
                .cast::<PyString>()
                .map_err(|_| ConstructionError::new_err("range bounds must be a string"))?
                .to_str()?,
        };
        let (lower_inc, upper_inc) = match bounds {
            "[)" => (true, false),
            "[]" => (true, true),
            "()" => (false, false),
            "(]" => (false, true),
            _ => {
                return Err(ConstructionError::new_err(
                    r#"range bounds must be "[)", "[]", "()" or "(]""#,
                ));
            }
        };
        Ok(Self::bounds(lower, lower_inc, upper, upper_inc))
    }

    /// The range containing no value.
    #[staticmethod]
    pub(crate) fn empty() -> Self {
        Self {
            lower: None,
            upper: None,
            lower_inc: false,
            upper_inc: false,
            empty: true,
        }
    }

    #[getter]
    fn is_empty(&self) -> bool {
        self.empty
    }

    #[getter]
    fn lower(&self, py: Python<'_>) -> Py<PyAny> {
        self.lower
            .as_ref()
            .map_or_else(|| py.None(), |v| v.clone_ref(py))
    }

    #[getter]
    fn upper(&self, py: Python<'_>) -> Py<PyAny> {
        self.upper
            .as_ref()
            .map_or_else(|| py.None(), |v| v.clone_ref(py))
    }

    #[getter(bounds)]
    pub(crate) fn bounds_text(&self) -> &'static str {
        match (self.lower_inc, self.upper_inc) {
            (true, true) => "[]",
            (true, false) => "[)",
            (false, true) => "(]",
            (false, false) => "()",
        }
    }

    #[getter]
    fn lower_inc(&self) -> bool {
        self.lower_inc
    }

    #[getter]
    fn upper_inc(&self) -> bool {
        self.upper_inc
    }

    #[getter]
    fn lower_inf(&self) -> bool {
        !self.empty && self.lower.is_none()
    }

    #[getter]
    fn upper_inf(&self) -> bool {
        !self.empty && self.upper.is_none()
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, op: CompareOp) -> PyResult<Py<PyAny>> {
        let py = other.py();
        let Ok(other) = other.cast::<Self>() else {
            return Ok(py.NotImplemented());
        };
        let equal = self.key(py)?.eq(other.get().key(py)?)?;
        match op {
            CompareOp::Eq => Ok(equal.into_pyobject(py)?.to_owned().into_any().unbind()),
            CompareOp::Ne => Ok((!equal).into_pyobject(py)?.to_owned().into_any().unbind()),
            _ => Ok(py.NotImplemented()),
        }
    }

    fn __hash__(&self, py: Python<'_>) -> PyResult<isize> {
        self.key(py)?.hash()
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        if self.empty {
            return Ok("Range.empty()".to_owned());
        }
        Ok(format!(
            "Range({}, {}, '{}')",
            self.lower(py).bind(py).repr()?,
            self.upper(py).bind(py).repr()?,
            self.bounds_text()
        ))
    }
}

/// A multirange: an immutable sequence of ranges, in the order written. The
/// server stores one sorted and merged.
// [spec:pgorm:req:bindings.values]
// [spec:pgorm:req:python.values+7]
#[pyclass(name = "Multirange", module = "pgorm", frozen, sequence, generic)]
#[derive(Debug)]
pub struct PyMultirange {
    pub(crate) ranges: Vec<Py<PyRange>>,
}

#[pymethods]
impl PyMultirange {
    #[new]
    #[pyo3(signature = (ranges=None))]
    fn new(ranges: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let ranges = match ranges {
            None => Vec::new(),
            Some(ranges) => ranges
                .try_iter()?
                .map(|item| {
                    item?
                        .cast_into::<PyRange>()
                        .map(Bound::unbind)
                        .map_err(|_| ConstructionError::new_err("a multirange holds Range values"))
                })
                .collect::<PyResult<_>>()?,
        };
        Ok(Self { ranges })
    }

    fn __len__(&self) -> usize {
        self.ranges.len()
    }

    fn __getitem__(&self, py: Python<'_>, key: isize) -> PyResult<Py<PyRange>> {
        let len = self.ranges.len() as isize;
        let at = if key < 0 { key + len } else { key };
        usize::try_from(at)
            .ok()
            .and_then(|at| self.ranges.get(at))
            .map(|range| range.clone_ref(py))
            .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err("multirange index out of range"))
    }

    fn __iter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let items = PyTuple::new(py, self.ranges.iter().map(|range| range.clone_ref(py)))?;
        Ok(items.try_iter()?.into_any().unbind())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, op: CompareOp) -> PyResult<Py<PyAny>> {
        let py = other.py();
        let Ok(other) = other.cast::<Self>() else {
            return Ok(py.NotImplemented());
        };
        let equal = self.items(py)?.eq(other.get().items(py)?)?;
        match op {
            CompareOp::Eq => Ok(equal.into_pyobject(py)?.to_owned().into_any().unbind()),
            CompareOp::Ne => Ok((!equal).into_pyobject(py)?.to_owned().into_any().unbind()),
            _ => Ok(py.NotImplemented()),
        }
    }

    fn __hash__(&self, py: Python<'_>) -> PyResult<isize> {
        self.items(py)?.hash()
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!("Multirange({})", self.items(py)?.repr()?))
    }
}

impl PyMultirange {
    fn items<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.ranges.iter().map(|range| range.clone_ref(py)))
    }
}
