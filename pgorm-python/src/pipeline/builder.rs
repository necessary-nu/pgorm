use pgorm::pipeline as pl;
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyTuple};

use super::{
    bound::{self, Stage},
    construct::integer,
    expression::PyPipelineExpr,
    scope::{Plan, callback},
    source::pipeline_source,
    window::{PyOver, plan},
};
use crate::{
    errors::ConstructionError, execution::Executor, expressions::Compiled, results,
    statements::Join,
};

// [spec:pgorm:req:bindings.pipeline]
/// An immutable wrapper owning the real Rust Pipeline, including its bound values.
#[pyclass(name = "Pipeline", module = "pgorm.pipeline", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyPipeline {
    pub inner: pl::Pipeline,
}

pub(super) fn join_side(kind: Join) -> pl::JoinSide {
    match kind {
        Join::Inner => pl::JoinSide::Inner,
        Join::Left => pl::JoinSide::Left,
        Join::Right => pl::JoinSide::Right,
        Join::Full => pl::JoinSide::Full,
    }
}

impl PyPipeline {
    pub(crate) fn compile(&self, terminal: &str) -> PyResult<Compiled> {
        let pipeline = match terminal {
            "all" => self.inner.clone(),
            "one" | "optional" => self.inner.clone().take(1),
            _ => return Err(PyTypeError::new_err("unknown pipeline terminal")),
        };
        let (sql, values) = pipeline
            .into_sql()
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        if values.0.len() > 65535 {
            return Err(ConstructionError::new_err(
                "PostgreSQL supports at most 65535 query parameters",
            ));
        }
        Ok(Compiled { sql, values })
    }

    /// `Pipeline::all`, `one` or `one_opt`: the latter two compile with the
    /// `take 1` Rust appends, and every row decodes as a statement's does.
    // [spec:pgorm:req:bindings.pipeline-terminals]
    fn read<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
        terminal: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let compiled = Py::new(py, self.compile(terminal)?)?
            .into_bound(py)
            .into_any();
        results::fetch(py, Executor::extract(py, connection)?, &compiled, terminal)
    }
}

#[pymethods]
impl PyPipeline {
    #[new]
    fn new(source: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: pipeline_source(source)?.pipeline(),
        })
    }

    fn filter(&self, condition: &Bound<'_, PyAny>) -> PyResult<Self> {
        let plan = Plan::plain(vec![PyPipelineExpr::coerce(condition)?])?;
        Ok(Self {
            inner: bound::filter(self.inner.clone(), plan)?,
        })
    }

    fn filter_with(&self, function: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: bound::filter(self.inner.clone(), callback(function, true)?)?,
        })
    }

    #[pyo3(signature = (*columns))]
    fn derive(&self, columns: &Bound<'_, PyTuple>) -> PyResult<Self> {
        Ok(Self {
            inner: bound::apply(self.inner.clone(), Stage::Derive, plan(columns)?)?,
        })
    }

    fn derive_with(&self, function: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: bound::apply(
                self.inner.clone(),
                Stage::Derive,
                callback(function, false)?,
            )?,
        })
    }

    #[pyo3(signature = (*columns))]
    fn select(&self, columns: &Bound<'_, PyTuple>) -> PyResult<Self> {
        Ok(Self {
            inner: bound::apply(self.inner.clone(), Stage::Select, plan(columns)?)?,
        })
    }

    fn select_with(&self, function: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: bound::apply(
                self.inner.clone(),
                Stage::Select,
                callback(function, false)?,
            )?,
        })
    }

    #[pyo3(signature = (*keys))]
    fn group(&self, keys: &Bound<'_, PyTuple>) -> PyResult<PyGrouped> {
        Ok(PyGrouped {
            inner: bound::grouped(self.inner.clone(), plan(keys)?)?,
        })
    }

    fn group_with(&self, function: &Bound<'_, PyAny>) -> PyResult<PyGrouped> {
        Ok(PyGrouped {
            inner: bound::grouped(self.inner.clone(), callback(function, false)?)?,
        })
    }

    #[pyo3(signature = (*columns, over))]
    fn window(&self, columns: &Bound<'_, PyTuple>, over: PyRef<'_, PyOver>) -> PyResult<Self> {
        Ok(Self {
            inner: bound::apply(
                self.inner.clone(),
                Stage::Window(over.inner.clone()),
                plan(columns)?,
            )?,
        })
    }

    fn window_with(&self, over: PyRef<'_, PyOver>, function: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: bound::apply(
                self.inner.clone(),
                Stage::Window(over.inner.clone()),
                callback(function, false)?,
            )?,
        })
    }

    #[pyo3(signature = (*keys))]
    fn sort(&self, keys: &Bound<'_, PyTuple>) -> PyResult<Self> {
        Ok(Self {
            inner: bound::apply(self.inner.clone(), Stage::Sort, plan(keys)?)?,
        })
    }

    fn sort_with(&self, function: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: bound::apply(self.inner.clone(), Stage::Sort, callback(function, false)?)?,
        })
    }

    fn take(&self, rows: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.clone().take(integer(rows)?),
        })
    }

    fn take_range(&self, start: &Bound<'_, PyAny>, end: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self
                .inner
                .clone()
                .take_range(integer(start)?..=integer(end)?),
        })
    }

    #[pyo3(signature = (source, on, *, kind=Join::Inner))]
    fn join(&self, source: &Bound<'_, PyAny>, on: &Bound<'_, PyAny>, kind: Join) -> PyResult<Self> {
        let source = pipeline_source(source)?.source();
        let plan = Plan::plain(vec![PyPipelineExpr::coerce(on)?])?;
        Ok(Self {
            inner: bound::join(self.inner.clone(), join_side(kind), source, plan)?,
        })
    }

    #[pyo3(signature = (source, function, *, kind=Join::Inner))]
    fn join_with(
        &self,
        source: &Bound<'_, PyAny>,
        function: &Bound<'_, PyAny>,
        kind: Join,
    ) -> PyResult<Self> {
        let source = pipeline_source(source)?.source();
        let plan = callback(function, true)?;
        Ok(Self {
            inner: bound::join(self.inner.clone(), join_side(kind), source, plan)?,
        })
    }

    fn append(&self, source: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.clone().append(pipeline_source(source)?.source()),
        })
    }

    fn intersect(&self, source: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self
                .inner
                .clone()
                .intersect(pipeline_source(source)?.source()),
        })
    }

    fn remove(&self, source: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.clone().remove(pipeline_source(source)?.source()),
        })
    }

    fn distinct(&self) -> Self {
        Self {
            inner: self.inner.clone().distinct(),
        }
    }

    // [spec:pgorm:req:bindings.pipeline-terminals]
    #[pyo3(signature = (*, terminal="all"))]
    fn inspect(&self, terminal: &str) -> PyResult<Compiled> {
        self.compile(terminal)
    }

    #[pyo3(signature = (selection, *, qualifiers=None))]
    fn select_sources(
        &self,
        selection: PyRef<'_, super::PySourceSelection>,
        qualifiers: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<super::PySelectedSources> {
        selection.select(self, qualifiers)
    }

    fn all<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.read(py, connection, "all")
    }

    fn one<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.read(py, connection, "one")
    }

    fn optional<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.read(py, connection, "optional")
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "pipelines cannot be tested as Python booleans",
        ))
    }
}

#[pyclass(
    name = "PipelineGrouped",
    module = "pgorm.pipeline",
    frozen,
    from_py_object
)]
#[derive(Clone, Debug)]
pub struct PyGrouped {
    inner: pl::Grouped,
}

#[pymethods]
impl PyGrouped {
    #[pyo3(signature = (*columns))]
    fn aggregate(&self, columns: &Bound<'_, PyTuple>) -> PyResult<PyPipeline> {
        Ok(PyPipeline {
            inner: bound::aggregate(self.inner.clone(), plan(columns)?)?,
        })
    }

    fn aggregate_with(&self, function: &Bound<'_, PyAny>) -> PyResult<PyPipeline> {
        Ok(PyPipeline {
            inner: bound::aggregate(self.inner.clone(), callback(function, false)?)?,
        })
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "grouped pipelines require aggregation",
        ))
    }
}
