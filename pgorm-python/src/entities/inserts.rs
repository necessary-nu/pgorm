//! A registered entity's insert, typed by its row count as Rust's is: an
//! `EntityInsert` is `Insert<A, OneRow>` and an `EntityInsertMany`
//! `Insert<A, ManyRows>`, each with its own count's terminals. They are
//! statement terminals, as in Rust, so no ActiveModel hook runs around them.

use std::sync::Arc;

use crate::panics::future_into_py;
use pgorm::{ActiveValue, pgorm_query::OnConflict, pgorm_query::Value};
use pyo3::{
    IntoPyObjectExt,
    prelude::*,
    types::{PyList, PyTuple},
};

use super::{
    PyActiveModel,
    backend::{Active, EntityBackend, InsertAnswer, Insertion, Insertions},
    versions::{model, owned, upserted},
};
use crate::{
    errors::{ConstructionError, InternalError},
    execution::Executor,
    statements::conflict_clause,
    transactions::work::{Output, Work},
};

/// Each terminal's capability name and the Rust API it calls.
pub(crate) const TERMINALS: [(&str, &str); 8] = [
    (
        "entity.insert.execute",
        "pgorm::Insert::one, Insert::on_conflict, Insert::exec",
    ),
    (
        "entity.insert.returning_pk",
        "pgorm::Insert::one, Insert::on_conflict, Insert<A, OneRow>::exec_returning_pk",
    ),
    (
        "entity.insert.returning_model",
        "pgorm::Insert::one, Insert::on_conflict, Insert<A, OneRow>::exec_returning_model",
    ),
    (
        "entity.insert.returning_upsert",
        "pgorm::Insert::one, Insert::on_conflict, Insert<A, OneRow>::exec_returning_upsert, pgorm::Upserted",
    ),
    (
        "entity.insert_many.execute",
        "pgorm::Insert::many, Insert::{add, add_many, on_conflict}, Insert::exec",
    ),
    (
        "entity.insert_many.returning_pks",
        "pgorm::Insert::many, Insert::{add, add_many, on_conflict}, Insert<A, ManyRows>::exec_returning_pks",
    ),
    (
        "entity.insert_many.returning_models",
        "pgorm::Insert::many, Insert::{add, add_many, on_conflict}, Insert<A, ManyRows>::exec_returning_models",
    ),
    (
        "entity.insert_many.returning_upserts",
        "pgorm::Insert::many, Insert::{add, add_many, on_conflict}, Insert<A, ManyRows>::exec_returning_upserts, pgorm::Upserted",
    ),
];

/// The columns `active` holds a value for, set or unchanged: the ones an
/// insert of it writes, as `Insert` reads an ActiveModel.
fn present(entity: &dyn EntityBackend, active: &Active) -> Vec<bool> {
    entity
        .info()
        .columns
        .iter()
        .map(|column| {
            matches!(
                active.get(&column.name),
                Ok(ActiveValue::Set(_) | ActiveValue::Unchanged(_))
            )
        })
        .collect()
}

/// Refuse a batch whose ActiveModels do not all set the same columns, which
/// Rust's `Insert` records as they are added and refuses before anything is
/// sent; here as they are given.
// [spec:pgorm:req:bindings.entity-inserts]
fn uniform(entity: &dyn EntityBackend, actives: &[Active]) -> PyResult<()> {
    let mut sets = actives.iter().map(|active| present(entity, active));
    let Some(first) = sets.next() else {
        return Ok(());
    };
    let Some(later) = sets.find(|set| *set != first) else {
        return Ok(());
    };
    let named = |this: &[bool], other: &[bool]| {
        entity
            .info()
            .columns
            .iter()
            .zip(this.iter().zip(other))
            .filter(|(_, (this, other))| **this && !**other)
            .map(|(column, _)| format!("{:?}", column.name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let sides = [
        (
            named(&first, &later),
            "set in the first ActiveModel but not in a later one",
        ),
        (
            named(&later, &first),
            "set in a later ActiveModel but not in the first",
        ),
    ]
    .into_iter()
    .filter(|(names, _)| !names.is_empty())
    .map(|(names, side)| format!("{names} {side}"))
    .collect::<Vec<_>>();
    Err(ConstructionError::new_err(format!(
        "the ActiveModels of one insert set the same columns: {}",
        sides.join("; ")
    )))
}

/// An insert's ActiveModels and its optional `ON CONFLICT` action.
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
            .collect::<PyResult<Vec<_>>>()?;
        uniform(entity.as_ref(), &actives)?;
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

    /// The batch of its ActiveModels and then `actives`, its conflict clause
    /// kept, as Rust's `add` and `add_many` make one.
    fn add(&self, actives: &[PyRef<'_, PyActiveModel>]) -> PyResult<PyEntityInsertMany> {
        let mut next = self.clone();
        for active in actives {
            next.actives.push(owned(&self.entity, active)?);
        }
        uniform(next.entity.as_ref(), &next.actives)?;
        Ok(PyEntityInsertMany { inner: next })
    }

    fn run<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
        one: bool,
        answer: InsertAnswer,
    ) -> PyResult<Bound<'py, PyAny>> {
        let target = Executor::extract(py, connection)?;
        let entity = self.entity.clone();
        let insertion = Insertion {
            actives: self.actives.clone(),
            conflict: self.conflict.clone(),
            one,
            answer,
        };
        future_into_py(py, async move {
            let work = Work::Insert(entity.clone(), Box::new(insertion));
            let Output::Inserted(inserted) = target.run(work).await? else {
                return Err(InternalError::new_err("unexpected insert result"));
            };
            Python::attach(|py| answered(py, entity.as_ref(), inserted, one))
        })
    }
}

/// A written row's key as Rust's `PrimaryKey::ValueType` holds it: the
/// value of a key of one column, a tuple of a composite key's.
fn key(py: Python<'_>, entity: &dyn EntityBackend, values: Vec<Value>) -> PyResult<Py<PyAny>> {
    let info = entity.info();
    let mut parts = info
        .primary_key
        .iter()
        .zip(values)
        .map(|(name, value)| info.column(name)?.input.tagged(value)?.to_python(py))
        .collect::<PyResult<Vec<_>>>()?;
    if parts.len() == 1
        && let Some(part) = parts.pop()
    {
        return Ok(part);
    }
    Ok(PyTuple::new(py, parts)?.into_any().unbind())
}

/// What the terminal answers: one row's or none for an insert of one, a list
/// for a batch.
fn answered(
    py: Python<'_>,
    entity: &dyn EntityBackend,
    inserted: Insertions,
    one: bool,
) -> PyResult<Py<PyAny>> {
    let items = match inserted {
        Insertions::Count(count) => return count.into_py_any(py),
        Insertions::Keys(keys) => keys
            .into_iter()
            .map(|values| key(py, entity, values))
            .collect::<PyResult<Vec<_>>>()?,
        Insertions::Models(models) => models
            .into_iter()
            .map(|inner| model(py, inner)?.into_py_any(py))
            .collect::<PyResult<Vec<_>>>()?,
        Insertions::Upserts(rows) => rows
            .into_iter()
            .map(|row| upserted(py, row)?.into_py_any(py))
            .collect::<PyResult<Vec<_>>>()?,
    };
    if one {
        return Ok(items.into_iter().next().unwrap_or_else(|| py.None()));
    }
    Ok(PyList::new(py, items)?.into_any().unbind())
}

// [spec:pgorm:req:bindings.entity-versions+3]
// [spec:pgorm:req:bindings.entity-inserts]
/// `Insert::one` of a registration's ActiveModel, with an optional `ON
/// CONFLICT` action: Rust's `Insert<A, OneRow>`.
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

    /// A batch of its ActiveModel and `active`, as `Insert::add` makes one.
    fn add(&self, active: PyRef<'_, PyActiveModel>) -> PyResult<PyEntityInsertMany> {
        self.inner.add(&[active])
    }

    /// A batch of its ActiveModel and `actives`, as `Insert::add_many` makes
    /// one.
    fn add_many(&self, actives: Vec<PyRef<'_, PyActiveModel>>) -> PyResult<PyEntityInsertMany> {
        self.inner.add(&actives)
    }

    /// How many rows the insert wrote, by `Insert::exec`.
    fn execute<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner.run(py, connection, true, InsertAnswer::Count)
    }

    /// The written row's key, by `Insert::exec_returning_pk`; a row the
    /// conflict clause held back is a `DecodeError`.
    fn returning_pk<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner.run(py, connection, true, InsertAnswer::Keys)
    }

    /// The written row, by `Insert::exec_returning_model`; a row the conflict
    /// clause held back is a `DecodeError`.
    fn returning_model<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner.run(py, connection, true, InsertAnswer::Models)
    }

    /// What the insert did with its row: `Upserted.Inserted`, or
    /// `Upserted.Updated` with the row before and after; `None` for a row the
    /// conflict clause did not write.
    fn returning_upsert<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner.run(py, connection, true, InsertAnswer::Upserts)
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity writes cannot be tested as Python booleans",
        ))
    }
}

// [spec:pgorm:req:bindings.entity-versions+3]
// [spec:pgorm:req:bindings.entity-inserts]
/// `Insert::many` of a registration's ActiveModels, with an optional `ON
/// CONFLICT` action: Rust's `Insert<A, ManyRows>`.
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

    /// The batch with `active` after its own, as `Insert::add` adds one.
    fn add(&self, active: PyRef<'_, PyActiveModel>) -> PyResult<Self> {
        self.inner.add(&[active])
    }

    /// The batch with `actives` after its own, as `Insert::add_many` adds
    /// them.
    fn add_many(&self, actives: Vec<PyRef<'_, PyActiveModel>>) -> PyResult<Self> {
        self.inner.add(&actives)
    }

    /// How many rows the insert wrote, by `Insert::exec`; a batch of none
    /// sends nothing.
    fn execute<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner.run(py, connection, false, InsertAnswer::Count)
    }

    /// Each written row's key, in the order the database wrote them, by
    /// `Insert::exec_returning_pks`; a row the conflict clause held back is
    /// left out.
    fn returning_pks<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner.run(py, connection, false, InsertAnswer::Keys)
    }

    /// Each written row, in the order the database wrote them, by
    /// `Insert::exec_returning_models`; a row the conflict clause held back is
    /// left out.
    fn returning_models<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner.run(py, connection, false, InsertAnswer::Models)
    }

    /// What the insert did with each row it wrote, in the order the database
    /// wrote them; a row the conflict clause skipped is not among them, and a
    /// batch of no models writes and answers nothing.
    fn returning_upserts<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner.run(py, connection, false, InsertAnswer::Upserts)
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity writes cannot be tested as Python booleans",
        ))
    }
}
