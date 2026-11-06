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
    statements::Join, values::PyTypeName,
};
use pgorm::pgorm_query::Name;

// [spec:pgorm:req:bindings.pipeline+1]
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

    /// A registered source tuple's entities (`bindings.pipeline-sources`), or
    /// declared models' records, one model or more each restating a relation
    /// the pipeline read (`bindings.pipeline-models`).
    #[pyo3(signature = (selection, *models, qualifiers=None))]
    fn select_sources<'py>(
        &self,
        py: Python<'py>,
        selection: &Bound<'py, PyAny>,
        models: &Bound<'py, PyTuple>,
        qualifiers: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if let Ok(registered) = selection.extract::<PyRef<'_, super::PySourceSelection>>() {
            if !models.is_empty() {
                return Err(PyTypeError::new_err(
                    "select_sources takes one registered source tuple, or models alone",
                ));
            }
            return Ok(Py::new(py, registered.select(self, qualifiers)?)?
                .into_bound(py)
                .into_any());
        }
        if qualifiers.is_some_and(|value| !value.is_none()) {
            return Err(PyTypeError::new_err(
                "qualifiers name a registered source tuple's relations; a model is read by its own name or alias",
            ));
        }
        let mut sources = vec![selection.clone()];
        sources.extend(models.iter());
        typed(py, "select_models", (self.clone(), sources))
    }

    /// The rows staged as a model's records or a function's results, as
    /// Rust's `into_model::<M>` stages them (`bindings.pipeline-models`).
    // [spec:pgorm:req:bindings.pipeline-models]
    #[pyo3(name = "into_model")]
    fn staged_model<'py>(
        &self,
        py: Python<'py>,
        target: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        typed(py, "into_model", (self.clone(), target))
    }

    /// The rows staged as tuples read by position, as Rust's `into_tuple`
    /// stages them (`bindings.pipeline-models`).
    // [spec:pgorm:req:bindings.pipeline-models]
    #[pyo3(name = "into_tuple")]
    fn staged_tuple<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        typed(py, "into_tuple", (self.clone(),))
    }

    /// pgorm's `select_described`: the projection of each declared model
    /// `sources` describes, its qualifier and each column with the type it is
    /// read as where its record reads it cast.
    // [spec:pgorm:req:bindings.pipeline-models]
    #[allow(clippy::type_complexity)]
    fn _select_described(
        &self,
        sources: Vec<(String, Vec<(String, Option<PyRef<'_, PyTypeName>>, bool)>)>,
    ) -> PyResult<Self> {
        let sources = sources.into_iter().map(|(qualifier, columns)| {
            columns.into_iter().fold(
                pl::DescribedSource::new(Name::runtime(qualifier)),
                |source, (column, read_as, array)| {
                    let read_as = read_as.map(|name| match array {
                        true => name.rust_type().array(),
                        false => name.rust_type(),
                    });
                    source.column(Name::runtime(column), read_as)
                },
            )
        });
        let inner = self
            .inner
            .clone()
            .select_described(sources.collect::<Vec<_>>())
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        Ok(Self { inner })
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

/// The staged read `pgorm.pipeline` makes of `arguments` through its helper
/// `name`, which decodes the rows as records of the binding's models.
fn typed<'py>(
    py: Python<'py>,
    name: &str,
    arguments: impl pyo3::call::PyCallArgs<'py>,
) -> PyResult<Bound<'py, PyAny>> {
    py.import("pgorm.pipeline._typed")?
        .getattr(name)?
        .call1(arguments)
}

/// The rows `statement` reads, each a tuple of its values in column order,
/// for a staged `into_tuple()` read.
// [spec:pgorm:req:bindings.pipeline-models]
#[pyfunction]
pub(super) fn pipeline_tuples<'py>(
    py: Python<'py>,
    connection: &Bound<'_, PyAny>,
    statement: &Bound<'_, PyAny>,
    terminal: &str,
) -> PyResult<Bound<'py, PyAny>> {
    results::fetch_tuples(py, Executor::extract(py, connection)?, statement, terminal)
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
