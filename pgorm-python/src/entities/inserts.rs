//! A registered entity's insert, typed by its row count as Rust's is: an
//! `EntityInsert` is `Insert<A, OneRow>` and an `EntityInsertMany`
//! `Insert<A, ManyRows>`, each with its own count's terminals, and their
//! `on_conflict_do_nothing()` and `on_empty_do_nothing()` the
//! `EntityTryInsert` and `EntityTryInsertMany` that are `TryInsert<A, OneRow>`
//! and `TryInsert<A, ManyRows>`. They are statement terminals, as in Rust, so
//! no ActiveModel hook runs around them. pgorm-bindings builds and runs each
//! statement, so JavaScript's inserts are the same Rust.

use std::sync::Arc;

use crate::panics::future_into_py;
use pgorm::{
    ActiveValue, TryInsertResult,
    pgorm_query::{OnConflict, ValueTuple},
};
use pgorm_bindings::{InsertConflict, InsertTerminal, InsertWrite, TryInsertTerminal};
use pyo3::{
    IntoPyObjectExt,
    prelude::*,
    types::{PyList, PyTuple},
};

use super::{
    PyActiveModel,
    backend::{Active, EntityBackend, Insertion, Insertions},
    versions::{model, owned, upserted},
};
use crate::{
    errors::{ConstructionError, InternalError},
    execution::Executor,
    statements::conflict_clause,
    transactions::work::{Output, Work},
};

/// Each terminal's capability name and the Rust API it calls.
pub(crate) const TERMINALS: [(&str, &str); 14] = [
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
    (
        "entity.try_insert.execute",
        "pgorm::Insert::one, Insert::{on_conflict_do_nothing, on_empty_do_nothing}, TryInsert::{on_conflict, exec}, pgorm::TryInsertResult",
    ),
    (
        "entity.try_insert.returning_pk",
        "pgorm::Insert::one, Insert::{on_conflict_do_nothing, on_empty_do_nothing}, TryInsert<A, OneRow>::exec_returning_pk, pgorm::TryInsertResult",
    ),
    (
        "entity.try_insert.returning_model",
        "pgorm::Insert::one, Insert::{on_conflict_do_nothing, on_empty_do_nothing}, TryInsert<A, OneRow>::exec_returning_model, pgorm::TryInsertResult",
    ),
    (
        "entity.try_insert_many.execute",
        "pgorm::Insert::many, Insert::{on_conflict_do_nothing, on_empty_do_nothing}, TryInsert::{add, add_many, on_conflict, exec}, pgorm::TryInsertResult",
    ),
    (
        "entity.try_insert_many.returning_pks",
        "pgorm::Insert::many, Insert::{on_conflict_do_nothing, on_empty_do_nothing}, TryInsert<A, ManyRows>::exec_returning_pks, pgorm::TryInsertResult",
    ),
    (
        "entity.try_insert_many.returning_models",
        "pgorm::Insert::many, Insert::{on_conflict_do_nothing, on_empty_do_nothing}, TryInsert<A, ManyRows>::exec_returning_models, pgorm::TryInsertResult",
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

/// An insert's ActiveModels and its conflict clause, `C`: an `ON CONFLICT`
/// action for an `Insert`, an `InsertConflict` for a `TryInsert`, whose
/// `on_conflict_do_nothing` names the key.
#[derive(Clone, Debug)]
struct Rows<C> {
    entity: Arc<dyn EntityBackend>,
    actives: Vec<Active>,
    conflict: Option<C>,
}

impl Rows<Box<OnConflict>> {
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

    fn insert<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
        one: bool,
        terminal: InsertTerminal,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.run(
            py,
            connection,
            one,
            InsertWrite::Insert(self.conflict.clone(), terminal),
        )
    }

    /// The insert as a try-insert: its conflict clause `DO NOTHING` on the
    /// key with `key`, as `on_conflict_do_nothing` makes it, or kept, as
    /// `on_empty_do_nothing` does.
    fn attempted(&self, key: bool) -> Rows<InsertConflict> {
        Rows {
            entity: self.entity.clone(),
            actives: self.actives.clone(),
            conflict: match key {
                true => Some(InsertConflict::PrimaryKey),
                false => self.conflict.clone().map(InsertConflict::Clause),
            },
        }
    }
}

impl<C: Clone> Rows<C> {
    /// The same insert with the conflict clause `action` reads as.
    fn on_conflict(
        &self,
        action: &Bound<'_, PyAny>,
        clause: fn(Box<OnConflict>) -> C,
    ) -> PyResult<Self> {
        let mut next = self.clone();
        next.conflict = Some(clause(Box::new(conflict_clause(action)?)));
        Ok(next)
    }

    /// The batch of its ActiveModels and then `actives`, its conflict clause
    /// kept, as Rust's `add` and `add_many` make one.
    fn added(&self, actives: &[PyRef<'_, PyActiveModel>]) -> PyResult<Self> {
        let mut next = self.clone();
        for active in actives {
            next.actives.push(owned(&self.entity, active)?);
        }
        uniform(next.entity.as_ref(), &next.actives)?;
        Ok(next)
    }

    /// The insert, of one row when `one`, run as `write` says, a try-insert
    /// answering in a `TryInsertResult`.
    fn run<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
        one: bool,
        write: InsertWrite,
    ) -> PyResult<Bound<'py, PyAny>> {
        let target = Executor::extract(py, connection)?;
        let entity = self.entity.clone();
        let tried = matches!(write, InsertWrite::Try(..));
        let insertion = Insertion {
            models: self.actives.clone(),
            one,
            write,
        };
        future_into_py(py, async move {
            let work = Work::Insert(entity.clone(), Box::new(insertion));
            let Output::Inserted(inserted) = target.run(work).await? else {
                return Err(InternalError::new_err("unexpected insert result"));
            };
            Python::attach(|py| answered(py, entity.as_ref(), inserted, one, tried))
        })
    }
}

impl Rows<InsertConflict> {
    fn attempt<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
        one: bool,
        terminal: TryInsertTerminal,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.run(
            py,
            connection,
            one,
            InsertWrite::Try(self.conflict.clone(), terminal),
        )
    }
}

/// A written row's key as Rust's `PrimaryKey::ValueType` holds it: the
/// value of a key of one column, a tuple of a composite key's.
fn key(py: Python<'_>, entity: &dyn EntityBackend, values: ValueTuple) -> PyResult<Py<PyAny>> {
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

/// The `TryInsertResult` variant `name`, holding `value` if given: the class
/// a declared model's try-insert answers with too.
fn try_result(py: Python<'_>, name: &str, value: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
    let variant = py
        .import("pgorm.models.try_inserts")?
        .getattr("TryInsertResult")?
        .getattr(name)?;
    Ok(match value {
        Some(value) => variant.call1((value,))?,
        None => variant.call0()?,
    }
    .unbind())
}

/// What the terminal answers: one row's for an insert of one, or none where
/// its conflict clause held the row back; a list for a batch; and for a
/// try-insert, `tried`, that answer in a `TryInsertResult`.
fn answered(
    py: Python<'_>,
    entity: &dyn EntityBackend,
    result: Insertions,
    one: bool,
    tried: bool,
) -> PyResult<Py<PyAny>> {
    let inserted = match (result, tried) {
        (TryInsertResult::Inserted(inserted), _) => inserted,
        (TryInsertResult::Conflicted, true) => return try_result(py, "Conflicted", None),
        (TryInsertResult::Empty, true) => return try_result(py, "Empty", None),
        (_, false) => return Err(InternalError::new_err("an insert answered as a try-insert")),
    };
    let items = match inserted {
        pgorm_bindings::Inserted::Count(count) => {
            let count = count.into_py_any(py)?;
            return match tried {
                true => try_result(py, "Inserted", Some(count)),
                false => Ok(count),
            };
        }
        pgorm_bindings::Inserted::Keys(keys) => keys
            .into_iter()
            .map(|values| key(py, entity, values))
            .collect::<PyResult<Vec<_>>>()?,
        pgorm_bindings::Inserted::Models(models) => models
            .into_iter()
            .map(|inner| model(py, inner)?.into_py_any(py))
            .collect::<PyResult<Vec<_>>>()?,
        pgorm_bindings::Inserted::Upserts(rows) => rows
            .into_iter()
            .map(|row| upserted(py, row)?.into_py_any(py))
            .collect::<PyResult<Vec<_>>>()?,
    };
    let value = match one {
        true => items.into_iter().next().unwrap_or_else(|| py.None()),
        false => PyList::new(py, items)?.into_any().unbind(),
    };
    match tried {
        true => try_result(py, "Inserted", Some(value)),
        false => Ok(value),
    }
}

// [spec:pgorm:req:bindings.entity-versions+3]
// [spec:pgorm:req:bindings.entity-inserts]
/// `Insert::one` of a registration's ActiveModel, with an optional `ON
/// CONFLICT` action: Rust's `Insert<A, OneRow>`.
#[pyclass(name = "EntityInsert", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyEntityInsert {
    inner: Rows<Box<OnConflict>>,
}

impl PyEntityInsert {
    pub(super) fn new(
        entity: Arc<dyn EntityBackend>,
        active: PyRef<'_, PyActiveModel>,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: Rows::new(entity, &[active])?,
        })
    }
}

#[pymethods]
impl PyEntityInsert {
    /// The `ON CONFLICT` action: a `Conflict` or a `ConflictUpdate`.
    fn on_conflict(&self, action: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.on_conflict(action, |clause| clause)?,
        })
    }

    /// `ON CONFLICT` on the primary key `DO NOTHING`, as
    /// `Insert::on_conflict_do_nothing` makes it: an `EntityTryInsert`.
    // [spec:pgorm:req:bindings.entity-try-inserts]
    fn on_conflict_do_nothing(&self) -> PyEntityTryInsert {
        PyEntityTryInsert {
            inner: self.inner.attempted(true),
        }
    }

    /// The insert as an `EntityTryInsert`, its statement unchanged, as
    /// `Insert::on_empty_do_nothing` makes it.
    // [spec:pgorm:req:bindings.entity-try-inserts]
    fn on_empty_do_nothing(&self) -> PyEntityTryInsert {
        PyEntityTryInsert {
            inner: self.inner.attempted(false),
        }
    }

    /// A batch of its ActiveModel and `active`, as `Insert::add` makes one.
    fn add(&self, active: PyRef<'_, PyActiveModel>) -> PyResult<PyEntityInsertMany> {
        Ok(PyEntityInsertMany {
            inner: self.inner.added(&[active])?,
        })
    }

    /// A batch of its ActiveModel and `actives`, as `Insert::add_many` makes
    /// one.
    fn add_many(&self, actives: Vec<PyRef<'_, PyActiveModel>>) -> PyResult<PyEntityInsertMany> {
        Ok(PyEntityInsertMany {
            inner: self.inner.added(&actives)?,
        })
    }

    /// How many rows the insert wrote, by `Insert::exec`.
    fn execute<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner
            .insert(py, connection, true, InsertTerminal::Count)
    }

    /// The written row's key, by `Insert::exec_returning_pk`; a row the
    /// conflict clause held back is a `DecodeError`.
    fn returning_pk<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner
            .insert(py, connection, true, InsertTerminal::Keys)
    }

    /// The written row, by `Insert::exec_returning_model`; a row the conflict
    /// clause held back is a `DecodeError`.
    fn returning_model<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner
            .insert(py, connection, true, InsertTerminal::Models)
    }

    /// What the insert did with its row: `Upserted.Inserted`, or
    /// `Upserted.Updated` with the row before and after; `None` for a row the
    /// conflict clause did not write.
    fn returning_upsert<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner
            .insert(py, connection, true, InsertTerminal::Upserts)
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
    inner: Rows<Box<OnConflict>>,
}

impl PyEntityInsertMany {
    pub(super) fn new(
        entity: Arc<dyn EntityBackend>,
        actives: &[PyRef<'_, PyActiveModel>],
    ) -> PyResult<Self> {
        Ok(Self {
            inner: Rows::new(entity, actives)?,
        })
    }
}

#[pymethods]
impl PyEntityInsertMany {
    /// The `ON CONFLICT` action: a `Conflict` or a `ConflictUpdate`.
    fn on_conflict(&self, action: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.on_conflict(action, |clause| clause)?,
        })
    }

    /// `ON CONFLICT` on the primary key `DO NOTHING`, as
    /// `Insert::on_conflict_do_nothing` makes it: an `EntityTryInsertMany`.
    // [spec:pgorm:req:bindings.entity-try-inserts]
    fn on_conflict_do_nothing(&self) -> PyEntityTryInsertMany {
        PyEntityTryInsertMany {
            inner: self.inner.attempted(true),
        }
    }

    /// The batch as an `EntityTryInsertMany`, its statement unchanged, as
    /// `Insert::on_empty_do_nothing` makes it.
    // [spec:pgorm:req:bindings.entity-try-inserts]
    fn on_empty_do_nothing(&self) -> PyEntityTryInsertMany {
        PyEntityTryInsertMany {
            inner: self.inner.attempted(false),
        }
    }

    /// The batch with `active` after its own, as `Insert::add` adds one.
    fn add(&self, active: PyRef<'_, PyActiveModel>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.added(&[active])?,
        })
    }

    /// The batch with `actives` after its own, as `Insert::add_many` adds
    /// them.
    fn add_many(&self, actives: Vec<PyRef<'_, PyActiveModel>>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.added(&actives)?,
        })
    }

    /// How many rows the insert wrote, by `Insert::exec`; a batch of none
    /// sends nothing.
    fn execute<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner
            .insert(py, connection, false, InsertTerminal::Count)
    }

    /// Each written row's key, in the order the database wrote them, by
    /// `Insert::exec_returning_pks`; a row the conflict clause held back is
    /// left out.
    fn returning_pks<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner
            .insert(py, connection, false, InsertTerminal::Keys)
    }

    /// Each written row, in the order the database wrote them, by
    /// `Insert::exec_returning_models`; a row the conflict clause held back is
    /// left out.
    fn returning_models<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner
            .insert(py, connection, false, InsertTerminal::Models)
    }

    /// What the insert did with each row it wrote, in the order the database
    /// wrote them; a row the conflict clause skipped is not among them, and a
    /// batch of no models writes and answers nothing.
    fn returning_upserts<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner
            .insert(py, connection, false, InsertTerminal::Upserts)
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity writes cannot be tested as Python booleans",
        ))
    }
}

// [spec:pgorm:req:bindings.entity-try-inserts]
/// An entity's insert of one row that may write nothing, made by its
/// insert's `on_conflict_do_nothing()` or `on_empty_do_nothing()`: Rust's
/// `TryInsert<A, OneRow>`, whose terminals answer `TryInsertResult`.
#[pyclass(name = "EntityTryInsert", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyEntityTryInsert {
    inner: Rows<InsertConflict>,
}

#[pymethods]
impl PyEntityTryInsert {
    /// The `ON CONFLICT` action in place of the one it had, as
    /// `TryInsert::on_conflict` sets it.
    fn on_conflict(&self, action: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.on_conflict(action, InsertConflict::Clause)?,
        })
    }

    /// The `EntityTryInsertMany` of its ActiveModel and then `active`, its
    /// conflict clause kept, as `TryInsert::add` makes it.
    fn add(&self, active: PyRef<'_, PyActiveModel>) -> PyResult<PyEntityTryInsertMany> {
        Ok(PyEntityTryInsertMany {
            inner: self.inner.added(&[active])?,
        })
    }

    /// The `EntityTryInsertMany` of its ActiveModel and then `actives`, its
    /// conflict clause kept, as `TryInsert::add_many` makes it.
    fn add_many(&self, actives: Vec<PyRef<'_, PyActiveModel>>) -> PyResult<PyEntityTryInsertMany> {
        Ok(PyEntityTryInsertMany {
            inner: self.inner.added(&actives)?,
        })
    }

    /// The rows written, by `TryInsert::exec`: `Conflicted` when its conflict
    /// clause wrote none, `Empty` when its ActiveModel sets nothing.
    fn execute<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner
            .attempt(py, connection, true, TryInsertTerminal::Count)
    }

    /// The written row's key, by `TryInsert::exec_returning_pk`, or
    /// `Conflicted` or `Empty`.
    fn returning_pk<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner
            .attempt(py, connection, true, TryInsertTerminal::Keys)
    }

    /// The written row, by `TryInsert::exec_returning_model`, or
    /// `Conflicted` or `Empty`.
    fn returning_model<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner
            .attempt(py, connection, true, TryInsertTerminal::Models)
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity writes cannot be tested as Python booleans",
        ))
    }
}

// [spec:pgorm:req:bindings.entity-try-inserts]
/// An entity's insert of a batch that may write nothing, made by a batch's
/// `on_conflict_do_nothing()` or `on_empty_do_nothing()`, or a try-insert's
/// `add`: Rust's `TryInsert<A, ManyRows>`, whose terminals answer
/// `TryInsertResult`.
#[pyclass(name = "EntityTryInsertMany", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyEntityTryInsertMany {
    inner: Rows<InsertConflict>,
}

#[pymethods]
impl PyEntityTryInsertMany {
    /// The `ON CONFLICT` action in place of the one it had, as
    /// `TryInsert::on_conflict` sets it.
    fn on_conflict(&self, action: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.on_conflict(action, InsertConflict::Clause)?,
        })
    }

    /// The try-insert with `active` after its own rows, its conflict clause
    /// kept, as `TryInsert::add` adds it.
    fn add(&self, active: PyRef<'_, PyActiveModel>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.added(&[active])?,
        })
    }

    /// The try-insert with `actives` after its own rows, its conflict clause
    /// kept, as `TryInsert::add_many` adds them.
    fn add_many(&self, actives: Vec<PyRef<'_, PyActiveModel>>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.added(&actives)?,
        })
    }

    /// The rows written, by `TryInsert::exec`: `Conflicted` when its conflict
    /// clause wrote none of them, `Empty` for a batch of none.
    fn execute<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner
            .attempt(py, connection, false, TryInsertTerminal::Count)
    }

    /// Each written row's key in the order the database wrote them, by
    /// `TryInsert::exec_returning_pks`: `Conflicted` when its conflict clause
    /// held every row back, `Empty` for a batch of none.
    fn returning_pks<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner
            .attempt(py, connection, false, TryInsertTerminal::Keys)
    }

    /// Each written row in the order the database wrote them, by
    /// `TryInsert::exec_returning_models`: `Conflicted` when its conflict
    /// clause held every row back, `Empty` for a batch of none.
    fn returning_models<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inner
            .attempt(py, connection, false, TryInsertTerminal::Models)
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity writes cannot be tested as Python booleans",
        ))
    }
}
