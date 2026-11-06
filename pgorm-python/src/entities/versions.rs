//! The registered-entity updates that read a written row's two versions:
//! `UpdateOne::exec_returning_change` and `UpdateMany::exec_returning_changes`.
//! They are statement terminals, as in Rust, so no ActiveModel hook runs
//! around them; an insert's, its upserts' among them, are in `inserts`.

use std::sync::Arc;

use crate::panics::future_into_py;
use pgorm::pgorm_query::Condition;
use pyo3::{IntoPyObjectExt, exceptions::PyTypeError, prelude::*, types::PyList};

use super::{
    PyActiveModel, PyEntityModel,
    backend::{Active, Assignment, EntityBackend, VersionWrite, Versions},
};
use crate::{
    errors::{ConstructionError, InternalError},
    execution::Executor,
    expressions::PyExpr,
    statements::condition,
    transactions::work::{Output, Work},
};

/// Each terminal's capability name and the Rust API it calls.
pub(crate) const TERMINALS: [(&str, &str); 2] = [
    (
        "entity.update.returning_change",
        "pgorm::Update::one, UpdateOne::exec_returning_change, pgorm::Change",
    ),
    (
        "entity.update_many.returning_changes",
        "pgorm::Update::many, UpdateMany::{col_expr, filter, exec_returning_changes}, ColumnTrait::save_as",
    ),
];

// [spec:pgorm:req:bindings.entity-versions+3]
// [spec:pgorm:req:python.records+1]
/// A row as it was before a write and as the write left it: a registered
/// entity's two `EntityModel`s, a declared model's two `ModelRecord`s, or a
/// generated view's two models, each the record its write reads rows as.
#[pyclass(name = "Change", module = "pgorm", frozen, get_all, generic)]
#[derive(Debug)]
pub struct PyChange {
    old: Py<PyAny>,
    new: Py<PyAny>,
}

#[pymethods]
impl PyChange {
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!("Change({})", self.new.bind(py).repr()?))
    }
}

// [spec:pgorm:req:bindings.entity-versions+3]
// [spec:pgorm:req:python.records+1]
/// What an upsert did with a row it wrote: `Inserted(model)`, or
/// `Updated(change)` for the row its `ON CONFLICT DO UPDATE` updated, the
/// rows the record its write reads rows as.
#[pyclass(name = "Upserted", module = "pgorm", frozen)]
#[derive(Debug)]
pub enum PyUpserted {
    Inserted { model: Py<PyAny> },
    Updated { change: Py<PyChange> },
}

#[pymethods]
impl PyUpserted {
    /// The row as the statement left it, whichever it did.
    #[pyo3(name = "into_model")]
    fn left(&self, py: Python<'_>) -> Py<PyAny> {
        match self {
            Self::Inserted { model } => model.clone_ref(py),
            Self::Updated { change } => change.get().new.clone_ref(py),
        }
    }

    /// `Upserted[M]` at run time, a `types.GenericAlias` as `Change[M]` is:
    /// PyO3 makes no enum generic itself.
    #[classmethod]
    fn __class_getitem__<'py>(
        cls: &Bound<'py, pyo3::types::PyType>,
        rows: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        cls.py()
            .import("types")?
            .getattr("GenericAlias")?
            .call1((cls, rows))
    }
}

/// `_change(old, new)`: the `Change` a declared model's or a generated
/// view's write answers, of the records it reads rows as.
// [spec:pgorm:req:python.records+1]
#[pyfunction(name = "_change")]
pub(crate) fn make_change(old: Py<PyAny>, new: Py<PyAny>) -> PyChange {
    PyChange { old, new }
}

/// `_inserted(model)`: the `Upserted.Inserted` such a write answers.
// [spec:pgorm:req:python.records+1]
#[pyfunction(name = "_inserted")]
pub(crate) fn make_inserted(model: Py<PyAny>) -> PyUpserted {
    PyUpserted::Inserted { model }
}

/// `_updated(change)`: the `Upserted.Updated` such a write answers.
// [spec:pgorm:req:python.records+1]
#[pyfunction(name = "_updated")]
pub(crate) fn make_updated(change: Bound<'_, PyChange>) -> PyUpserted {
    PyUpserted::Updated {
        change: change.unbind(),
    }
}

pub(super) fn model(py: Python<'_>, inner: super::backend::Model) -> PyResult<PyEntityModel> {
    let model = PyEntityModel { inner };
    model.validate(py)?;
    Ok(model)
}

fn change(py: Python<'_>, versions: Versions) -> PyResult<PyChange> {
    let old = versions
        .old
        .ok_or_else(|| InternalError::new_err("an updated row returned no old row"))?;
    Ok(PyChange {
        old: model(py, old)?.into_py_any(py)?,
        new: model(py, versions.new)?.into_py_any(py)?,
    })
}

pub(super) fn upserted(py: Python<'_>, versions: Versions) -> PyResult<PyUpserted> {
    Ok(match versions.old {
        None => PyUpserted::Inserted {
            model: model(py, versions.new)?.into_py_any(py)?,
        },
        Some(old) => PyUpserted::Updated {
            change: Py::new(
                py,
                PyChange {
                    old: model(py, old)?.into_py_any(py)?,
                    new: model(py, versions.new)?.into_py_any(py)?,
                },
            )?,
        },
    })
}

/// How the terminal answers: one change, or every change.
#[derive(Clone, Copy)]
enum Answer {
    Change,
    Changes,
}

fn run<'py>(
    py: Python<'py>,
    connection: &Bound<'_, PyAny>,
    entity: Arc<dyn EntityBackend>,
    write: VersionWrite,
    answer: Answer,
) -> PyResult<Bound<'py, PyAny>> {
    let target = Executor::extract(py, connection)?;
    future_into_py(py, async move {
        let Output::Versions(rows) = target.run(Work::Versions(entity, Box::new(write))).await?
        else {
            return Err(InternalError::new_err("unexpected versioned write result"));
        };
        Python::attach(|py| match answer {
            Answer::Change => match rows.into_iter().next() {
                Some(row) => change(py, row)?.into_py_any(py),
                None => Err(InternalError::new_err("an update by key returned no row")),
            },
            Answer::Changes => {
                let changes = rows
                    .into_iter()
                    .map(|row| change(py, row))
                    .collect::<PyResult<Vec<_>>>()?;
                Ok(PyList::new(py, changes)?.into_any().unbind())
            }
        })
    })
}

/// An ActiveModel of this registration; another's is an argument of the
/// wrong type.
// [spec:pgorm:req:bindings.entity-writes+1]
pub(super) fn owned(entity: &Arc<dyn EntityBackend>, active: &PyActiveModel) -> PyResult<Active> {
    if active.inner.info().name != entity.info().name {
        return Err(PyTypeError::new_err(format!(
            "the ActiveModel is {}'s, not {}'s",
            active.inner.info().name,
            entity.info().name
        )));
    }
    Ok(active.inner.clone())
}

// [spec:pgorm:req:bindings.entity-versions+3]
/// `Entity::update(active)`: an update by the model's key.
#[pyclass(name = "EntityUpdate", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyEntityUpdate {
    entity: Arc<dyn EntityBackend>,
    active: Active,
}

impl PyEntityUpdate {
    pub(super) fn new(entity: Arc<dyn EntityBackend>, active: &PyActiveModel) -> PyResult<Self> {
        let active = owned(&entity, active)?;
        Ok(Self { entity, active })
    }
}

#[pymethods]
impl PyEntityUpdate {
    /// The row before and after the update. With nothing set it sends nothing
    /// and both versions are the row the key reads; a key matching no row is
    /// `DatabaseError`.
    fn returning_change<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let write = VersionWrite::Change(self.active.clone());
        run(py, connection, self.entity.clone(), write, Answer::Change)
    }

    // [spec:pgorm:req:python.truth-testing+4]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity writes cannot be tested as Python booleans",
        ))
    }
}

// [spec:pgorm:req:bindings.entity-versions+3]
/// `Entity::update_many()`: an update of every row its filters admit, which
/// runs only once `where` or `all_rows()` has said which rows those are.
#[pyclass(name = "EntityUpdateMany", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyEntityUpdateMany {
    entity: Arc<dyn EntityBackend>,
    assignments: Vec<(String, Assignment)>,
    filter: Condition,
    guarded: bool,
}

impl PyEntityUpdateMany {
    pub(super) fn new(entity: Arc<dyn EntityBackend>) -> Self {
        Self {
            entity,
            assignments: Vec::new(),
            filter: Condition::all(),
            guarded: false,
        }
    }
}

#[pymethods]
impl PyEntityUpdateMany {
    /// Set a column to a value, converted by the column's declared type and
    /// written through its `save_as`, or to an expression.
    fn set(&self, column: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let name = super::column::name(column, self.entity.info())?;
        let assignment = if let Ok(expression) = value.extract::<PyRef<'_, PyExpr>>() {
            Assignment::Expr(expression.inner.clone())
        } else {
            let value = self.entity.info().column(&name)?.input.coerce(value)?;
            Assignment::Value(value.rust_value().clone())
        };
        let mut next = self.clone();
        next.assignments.push((name, assignment));
        Ok(next)
    }

    fn r#where(&self, predicate: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        next.filter = next.filter.add(condition(predicate)?);
        next.guarded = true;
        Ok(next)
    }

    /// Every row, said outright.
    fn all_rows(&self) -> Self {
        let mut next = self.clone();
        next.guarded = true;
        next
    }

    /// Every row the update changed, before and after, in the order the
    /// server returned them. An update with neither `where` nor `all_rows()`,
    /// and one setting nothing (Rust's `NothingToSet`), are refused before
    /// anything is sent.
    // [spec:pgorm:req:bindings.entity-versions+3]
    fn returning_changes<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if !self.guarded {
            return Err(ConstructionError::new_err(
                "an update of many rows needs where(..) or all_rows() before it runs",
            ));
        }
        let write = VersionWrite::Changes(self.assignments.clone(), self.filter.clone());
        run(py, connection, self.entity.clone(), write, Answer::Changes)
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity writes cannot be tested as Python booleans",
        ))
    }
}
