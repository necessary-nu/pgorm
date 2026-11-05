//! The registered-entity writes that read a written row's two versions:
//! `UpdateOne::exec_returning_change`, `UpdateMany::exec_returning_changes`
//! and `Insert::exec_returning_upsert(s)`. They are statement terminals, as in
//! Rust, so no ActiveModel hook runs around them.

use std::sync::Arc;

use pgorm::pgorm_query::{Condition, OnConflict};
use pyo3::{IntoPyObjectExt, prelude::*, types::PyList};
use pyo3_async_runtimes::tokio::future_into_py;

use super::{
    PyActiveModel, PyEntityModel,
    backend::{Active, Assignment, EntityBackend, VersionWrite, Versions},
};
use crate::{
    errors::{ConstructionError, InternalError, LifecycleError},
    execution::Executor,
    expressions::PyExpr,
    statements::{condition, conflict_clause},
    transactions::work::{Output, Work},
};

/// Each terminal's capability name and the Rust API it calls.
pub(crate) const TERMINALS: [(&str, &str); 4] = [
    (
        "entity.update.returning_change",
        "pgorm::Update::one, UpdateOne::exec_returning_change, pgorm::Change",
    ),
    (
        "entity.update_many.returning_changes",
        "pgorm::Update::many, UpdateMany::{col_expr, filter, exec_returning_changes}, ColumnTrait::save_as",
    ),
    (
        "entity.insert.returning_upsert",
        "pgorm::Insert::one, Insert::on_conflict, Insert<A, OneRow>::exec_returning_upsert, pgorm::Upserted",
    ),
    (
        "entity.insert_many.returning_upserts",
        "pgorm::Insert::many, Insert::on_conflict, Insert<A, ManyRows>::exec_returning_upserts, pgorm::Upserted",
    ),
];

// [spec:pgorm:req:bindings.entity-versions+2]
// [spec:pgorm:req:python.records]
/// A row as it was before a write and as the write left it.
#[pyclass(name = "Change", module = "pgorm", frozen, get_all, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyChange {
    old: PyEntityModel,
    new: PyEntityModel,
}

#[pymethods]
impl PyChange {
    fn __repr__(&self) -> String {
        format!("Change({:?})", self.new.inner.info().name)
    }
}

// [spec:pgorm:req:bindings.entity-versions+2]
// [spec:pgorm:req:python.records]
/// What an upsert did with a row it wrote: `Inserted(model)`, or
/// `Updated(change)` for the row its `ON CONFLICT DO UPDATE` updated.
#[pyclass(name = "Upserted", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub enum PyUpserted {
    Inserted { model: PyEntityModel },
    Updated { change: PyChange },
}

#[pymethods]
impl PyUpserted {
    /// The row as the statement left it, whichever it did.
    #[pyo3(name = "into_model")]
    fn left(&self) -> PyEntityModel {
        match self {
            Self::Inserted { model } => model.clone(),
            Self::Updated { change } => change.new.clone(),
        }
    }
}

fn model(py: Python<'_>, inner: super::backend::Model) -> PyResult<PyEntityModel> {
    let model = PyEntityModel { inner };
    model.validate(py)?;
    Ok(model)
}

fn change(py: Python<'_>, versions: Versions) -> PyResult<PyChange> {
    let old = versions
        .old
        .ok_or_else(|| InternalError::new_err("an updated row returned no old row"))?;
    Ok(PyChange {
        old: model(py, old)?,
        new: model(py, versions.new)?,
    })
}

fn upserted(py: Python<'_>, versions: Versions) -> PyResult<PyUpserted> {
    Ok(match versions.old {
        None => PyUpserted::Inserted {
            model: model(py, versions.new)?,
        },
        Some(old) => PyUpserted::Updated {
            change: PyChange {
                old: model(py, old)?,
                new: model(py, versions.new)?,
            },
        },
    })
}

/// How the terminal answers: one change, every change, the one upsert or none,
/// or every upsert.
#[derive(Clone, Copy)]
enum Answer {
    Change,
    Changes,
    Upsert,
    Upserts,
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
            Answer::Upsert => match rows.into_iter().next() {
                Some(row) => upserted(py, row)?.into_py_any(py),
                None => Ok(py.None()),
            },
            Answer::Upserts => {
                let rows = rows
                    .into_iter()
                    .map(|row| upserted(py, row))
                    .collect::<PyResult<Vec<_>>>()?;
                Ok(PyList::new(py, rows)?.into_any().unbind())
            }
        })
    })
}

/// An ActiveModel of this registration, refused if another minted it.
fn owned(entity: &Arc<dyn EntityBackend>, active: &PyActiveModel) -> PyResult<Active> {
    if active.inner.info().name != entity.info().name {
        return Err(LifecycleError::new_err(
            "ActiveModel belongs to another entity registration",
        ));
    }
    Ok(active.inner.clone())
}

// [spec:pgorm:req:bindings.entity-versions+2]
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

// [spec:pgorm:req:bindings.entity-versions+2]
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
    // [spec:pgorm:req:bindings.entity-versions+2]
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

/// An insert's models and its optional `ON CONFLICT` action.
#[derive(Clone, Debug)]
struct Inserted {
    entity: Arc<dyn EntityBackend>,
    actives: Vec<Active>,
    conflict: Option<Box<OnConflict>>,
}

impl Inserted {
    fn new(entity: Arc<dyn EntityBackend>, actives: &[PyRef<'_, PyActiveModel>]) -> PyResult<Self> {
        let actives = actives
            .iter()
            .map(|active| owned(&entity, active))
            .collect::<PyResult<_>>()?;
        Ok(Self {
            entity,
            actives,
            conflict: None,
        })
    }

    fn on_conflict(&self, action: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        next.conflict = Some(Box::new(conflict_clause(action)?));
        Ok(next)
    }

    fn run<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
        one: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let write = VersionWrite::Upserts {
            actives: self.actives.clone(),
            conflict: self.conflict.clone(),
            one,
        };
        let answer = if one { Answer::Upsert } else { Answer::Upserts };
        run(py, connection, self.entity.clone(), write, answer)
    }
}

// [spec:pgorm:req:bindings.entity-versions+2]
/// `Insert::one` of a registration's ActiveModel, with an optional `ON
/// CONFLICT` action.
#[pyclass(name = "EntityInsert", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyEntityInsert {
    inner: Inserted,
}

impl PyEntityInsert {
    pub(super) fn new(
        entity: Arc<dyn EntityBackend>,
        active: PyRef<'_, PyActiveModel>,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: Inserted::new(entity, &[active])?,
        })
    }
}

#[pymethods]
impl PyEntityInsert {
    /// The `ON CONFLICT` action: a `Conflict` or a `ConflictUpdate`.
    fn on_conflict(&self, action: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.on_conflict(action)?,
        })
    }

    /// What the insert did with its row: `Upserted.Inserted`, or
    /// `Upserted.Updated` with the row before and after; `None` for a row the
    /// conflict clause did not write.
    fn returning_upsert<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner.run(py, connection, true)
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity writes cannot be tested as Python booleans",
        ))
    }
}

// [spec:pgorm:req:bindings.entity-versions+2]
/// `Insert::many` of a registration's ActiveModels, with an optional `ON
/// CONFLICT` action.
#[pyclass(name = "EntityInsertMany", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyEntityInsertMany {
    inner: Inserted,
}

impl PyEntityInsertMany {
    pub(super) fn new(
        entity: Arc<dyn EntityBackend>,
        actives: &[PyRef<'_, PyActiveModel>],
    ) -> PyResult<Self> {
        Ok(Self {
            inner: Inserted::new(entity, actives)?,
        })
    }
}

#[pymethods]
impl PyEntityInsertMany {
    /// The `ON CONFLICT` action: a `Conflict` or a `ConflictUpdate`.
    fn on_conflict(&self, action: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.on_conflict(action)?,
        })
    }

    /// What the insert did with each row it wrote, in the order the database
    /// wrote them; a row the conflict clause skipped is not among them, and a
    /// batch of no models writes and answers nothing.
    fn returning_upserts<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner.run(py, connection, false)
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity writes cannot be tested as Python booleans",
        ))
    }
}
