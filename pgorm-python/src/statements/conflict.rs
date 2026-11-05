use pgorm::pgorm_query::{
    ConflictConstraint, ConflictTarget, ConflictUpdate, Name, OnConflict, SimpleExpr,
};
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyString, PyTuple},
};

use super::common;
use crate::{
    errors::ConstructionError,
    expressions::{PyExpr, coerce},
    identifiers::Identifier,
};

/// A completed Rust ON CONFLICT clause.
#[pyclass(name = "Conflict", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyConflict {
    pub inner: OnConflict,
}

#[pymethods]
impl PyConflict {
    #[staticmethod]
    fn do_nothing() -> Self {
        Self {
            inner: OnConflict::do_nothing(),
        }
    }

    // [spec:pgorm:req:bindings.writes+2]
    // [spec:pgorm:req:bindings.conflict-arbiters]
    /// An inference target: at least one column name or index expression.
    #[staticmethod]
    #[pyo3(signature = (*targets))]
    fn on(targets: &Bound<'_, PyTuple>) -> PyResult<PyConflictTarget> {
        let mut target = None;
        for item in targets.iter() {
            target = Some(element(target, &item)?);
        }
        let target = target.ok_or_else(|| {
            ConstructionError::new_err("a conflict target names at least one column or expression")
        })?;
        Ok(PyConflictTarget {
            inner: Arbiter::Target(target),
        })
    }

    // [spec:pgorm:req:bindings.conflict-arbiters]
    /// The constraint named outright, `ON CONSTRAINT "name"`, which takes no
    /// partial-index predicate.
    #[staticmethod]
    fn on_constraint(name: &Bound<'_, PyAny>) -> PyResult<PyConflictTarget> {
        Ok(PyConflictTarget {
            inner: Arbiter::Constraint(OnConflict::constraint(Identifier::new(name)?.name())),
        })
    }

    // [spec:pgorm:req:python.truth-testing+4]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "conflict clauses cannot be tested as Python booleans",
        ))
    }
}

/// What a conflict is judged by: an inference target of columns and index
/// expressions, or a constraint named outright.
#[derive(Clone, Debug)]
enum Arbiter {
    Target(ConflictTarget),
    Constraint(ConflictConstraint),
}

impl Arbiter {
    fn do_nothing(self) -> OnConflict {
        match self {
            Self::Target(target) => target.do_nothing(),
            Self::Constraint(constraint) => constraint.do_nothing(),
        }
    }

    fn update_column(self, column: Name) -> ConflictUpdate {
        match self {
            Self::Target(target) => target.update_column(column),
            Self::Constraint(constraint) => constraint.update_column(column),
        }
    }

    fn value(self, column: Name, value: SimpleExpr) -> ConflictUpdate {
        match self {
            Self::Target(target) => target.value(column, value),
            Self::Constraint(constraint) => constraint.value(column, value),
        }
    }
}

fn element(target: Option<ConflictTarget>, item: &Bound<'_, PyAny>) -> PyResult<ConflictTarget> {
    if let Ok(expr) = item.extract::<PyRef<'_, PyExpr>>() {
        let expr = expr.inner.clone();
        return Ok(match target {
            Some(target) => target.and_expr(expr),
            None => OnConflict::expr(expr),
        });
    }
    if !item.is_instance_of::<PyString>() {
        return Err(PyTypeError::new_err(
            "a conflict target is a column name or an index expression",
        ));
    }
    let column = Identifier::new(item)?.name();
    Ok(match target {
        Some(target) => target.and_column(column),
        None => OnConflict::column(column),
    })
}

/// The columns an update sets from `EXCLUDED`: the first, which makes the
/// update, and the rest. An update naming none is refused.
fn excluded(columns: &Bound<'_, PyTuple>) -> PyResult<(Name, Vec<Name>)> {
    let mut names = columns
        .iter()
        .map(|column| Ok(Identifier::new(&column)?.name()))
        .collect::<PyResult<Vec<_>>>()?;
    if names.is_empty() {
        return Err(ConstructionError::new_err(
            "an update names at least one column",
        ));
    }
    let first = names.remove(0);
    Ok((first, names))
}

// [spec:pgorm:req:bindings.writes+2]
// [spec:pgorm:req:bindings.conflict-arbiters]
/// A conflict arbiter before choosing its action, made by `Conflict.on` or
/// `Conflict.on_constraint`.
#[pyclass(name = "ConflictTarget", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyConflictTarget {
    inner: Arbiter,
}

#[pymethods]
impl PyConflictTarget {
    // [spec:pgorm:req:bindings.conflict-arbiters]
    /// The partial index's predicate; an `ON CONSTRAINT` arbiter takes none.
    fn r#where(&self, predicate: &Bound<'_, PyAny>) -> PyResult<Self> {
        let predicate = common::condition(predicate)?;
        match &self.inner {
            Arbiter::Target(target) => Ok(Self {
                inner: Arbiter::Target(target.clone().cond_where(predicate)),
            }),
            Arbiter::Constraint(_) => Err(ConstructionError::new_err(
                "ON CONSTRAINT takes no predicate; a partial index's predicate goes with Conflict.on(..)",
            )),
        }
    }

    fn do_nothing(&self) -> PyConflict {
        PyConflict {
            inner: self.inner.clone().do_nothing(),
        }
    }

    /// Set columns from the row that failed to insert, `EXCLUDED`'s; at least
    /// one.
    #[pyo3(signature = (*columns))]
    fn update(&self, columns: &Bound<'_, PyTuple>) -> PyResult<PyConflictUpdate> {
        let (first, rest) = excluded(columns)?;
        Ok(PyConflictUpdate {
            inner: self.inner.clone().update_column(first).update_columns(rest),
        })
    }

    fn set(
        &self,
        column: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<PyConflictUpdate> {
        Ok(PyConflictUpdate {
            inner: self
                .inner
                .clone()
                .value(Identifier::new(column)?.name(), coerce(value)?.inner),
        })
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "conflict targets cannot be tested as Python booleans",
        ))
    }
}

/// A nonempty Rust DO UPDATE assignment set, with optional update predicate.
#[pyclass(name = "ConflictUpdate", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyConflictUpdate {
    pub inner: ConflictUpdate,
}

#[pymethods]
impl PyConflictUpdate {
    fn set(&self, column: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self
                .inner
                .clone()
                .value(Identifier::new(column)?.name(), coerce(value)?.inner),
        })
    }

    #[pyo3(signature = (*columns))]
    fn update(&self, columns: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let (first, rest) = excluded(columns)?;
        Ok(Self {
            inner: self.inner.clone().update_column(first).update_columns(rest),
        })
    }

    fn r#where(&self, predicate: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.clone().cond_where(common::condition(predicate)?),
        })
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "conflict clauses cannot be tested as Python booleans",
        ))
    }
}

pub(crate) fn clause(value: &Bound<'_, PyAny>) -> PyResult<OnConflict> {
    if let Ok(value) = value.extract::<PyRef<'_, PyConflict>>() {
        Ok(value.inner.clone())
    } else if let Ok(value) = value.extract::<PyRef<'_, PyConflictUpdate>>() {
        Ok(value.inner.clone().into())
    } else {
        Err(PyTypeError::new_err(
            "on_conflict requires a completed conflict action",
        ))
    }
}
