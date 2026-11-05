use pgorm::pgorm_query::IntoCondition;
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyInt, PyTuple},
};

use super::backend::{Change, Select, Terminal};
use crate::{
    errors::ConstructionError,
    expressions::{Compiled, OrderBy, PyCondition, PyExpr},
    statements::RawSQL,
    transactions::work::Work,
};

#[derive(Clone, Debug)]
#[pyclass(name = "EntityQuery", module = "pgorm", frozen, from_py_object)]
pub struct PyEntityQuery {
    pub(crate) inner: Select,
}

pub(crate) fn bound(value: Option<&Bound<'_, PyAny>>) -> PyResult<Option<u64>> {
    let Some(value) = value.filter(|v| !v.is_none()) else {
        return Ok(None);
    };
    if !value.is_exact_instance_of::<PyInt>() {
        return Err(PyTypeError::new_err("query bounds require an integer"));
    }
    let number: i64 = value
        .extract()
        .map_err(|_| ConstructionError::new_err("query bound exceeds i64"))?;
    u64::try_from(number)
        .map(Some)
        .map_err(|_| ConstructionError::new_err("query bounds must be nonnegative"))
}

#[pymethods]
impl PyEntityQuery {
    #[getter]
    fn entity_name(&self) -> &str {
        &self.inner.info().name
    }

    fn r#where(&self, predicate: &Bound<'_, PyAny>) -> PyResult<Self> {
        let condition = if let Ok(condition) = predicate.extract::<PyRef<'_, PyCondition>>() {
            condition.inner.clone()
        } else if let Ok(expr) = predicate.extract::<PyRef<'_, PyExpr>>() {
            expr.inner.clone().into_condition()
        } else {
            return Err(PyTypeError::new_err(
                "entity filters require an Expr or Condition",
            ));
        };
        Ok(Self {
            inner: self.inner.change(Change::Filter(condition)),
        })
    }

    #[pyo3(signature = (*ordering))]
    fn order_by(&self, ordering: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let mut inner = self.inner.clone();
        for order in ordering {
            let order = order
                .extract::<PyRef<'_, OrderBy>>()
                .map_err(|_| PyTypeError::new_err("order_by requires typed OrderBy values"))?;
            inner = inner.change(Change::Order(
                order.expr.inner.clone(),
                order.direction.rust_order(),
                order.nulls.as_ref().map(|n| n.rust_nulls()),
            ));
        }
        Ok(Self { inner })
    }

    #[pyo3(signature = (value=None))]
    fn limit(&self, value: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.change(Change::Limit(bound(value)?)),
        })
    }
    #[pyo3(signature = (value=None))]
    fn offset(&self, value: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.change(Change::Offset(bound(value)?)),
        })
    }

    #[pyo3(signature = (*, terminal="all"))]
    fn inspect(&self, terminal: &str) -> PyResult<Compiled> {
        let terminal = match terminal {
            "all" => Terminal::All,
            "one" => Terminal::One,
            "optional" => Terminal::Optional,
            _ => return Err(PyTypeError::new_err("unknown entity terminal")),
        };
        let (sql, values) = self.inner.compile(terminal);
        if values.0.len() > 65535 {
            return Err(ConstructionError::new_err(
                "PostgreSQL supports at most 65535 query parameters",
            ));
        }
        Ok(Compiled { sql, values })
    }

    fn all<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        super::io::select(py, connection, self.inner.clone(), Terminal::All)
    }
    fn one<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        super::io::select(py, connection, self.inner.clone(), Terminal::One)
    }
    fn optional<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        super::io::select(py, connection, self.inner.clone(), Terminal::Optional)
    }
    /// `statement`'s rows in place of this query's, decoded into the
    /// entity's models as Rust's `Select::from_raw_sql` decodes them.
    // [spec:pgorm:req:bindings.raw-models]
    #[pyo3(name = "from_raw_sql")]
    fn raw_sql(&self, statement: PyRef<'_, RawSQL>) -> PyResult<PyEntityRawQuery> {
        Ok(PyEntityRawQuery {
            inner: self.inner.clone(),
            statement: statement.inspect()?,
        })
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity queries cannot be tested as Python booleans",
        ))
    }
    fn __repr__(&self) -> String {
        format!("EntityQuery({:?})", self.entity_name())
    }
}

/// A raw statement whose rows decode into a registration's models.
#[derive(Clone, Debug)]
#[pyclass(name = "EntityRawQuery", module = "pgorm", frozen, from_py_object)]
pub struct PyEntityRawQuery {
    pub(crate) inner: Select,
    pub(crate) statement: Compiled,
}

impl PyEntityRawQuery {
    fn run<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
        terminal: Terminal,
    ) -> PyResult<Bound<'py, PyAny>> {
        let work = Work::EntityRaw(self.inner.clone(), self.statement.clone(), terminal);
        super::io::models(py, connection, work, terminal)
    }
}

#[pymethods]
impl PyEntityRawQuery {
    #[getter]
    fn entity_name(&self) -> &str {
        &self.inner.info().name
    }

    fn inspect(&self) -> Compiled {
        self.statement.clone()
    }

    fn all<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.run(py, connection, Terminal::All)
    }
    fn one<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.run(py, connection, Terminal::One)
    }
    fn optional<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.run(py, connection, Terminal::Optional)
    }
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity queries cannot be tested as Python booleans",
        ))
    }
    fn __repr__(&self) -> String {
        format!("EntityRawQuery({:?})", self.entity_name())
    }
}
