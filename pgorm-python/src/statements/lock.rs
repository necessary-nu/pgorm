//! A SELECT's row lock, over pgorm-query's own lock builder: one of the four
//! strengths, `OF` the FROM items named, waiting as `NOWAIT` or `SKIP LOCKED`
//! says.

use pgorm::pgorm_query::{LockBehavior, LockType, Name, SelectStatement};
use pyo3::{exceptions::PyTypeError, prelude::*};

use super::from_item::from_item;
use crate::errors::ConstructionError;

/// How strongly the rows a SELECT reads are locked.
#[pyclass(name = "LockStrength", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PyLockStrength {
    Update,
    NoKeyUpdate,
    Share,
    KeyShare,
}

#[pymethods]
impl PyLockStrength {
    // [spec:pgorm:req:python.truth-testing+5]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "lock strengths cannot be tested as Python booleans",
        ))
    }
}

/// What a lock does with a row another transaction holds: fail at once
/// (`Nowait`) or leave the row out (`SkipLocked`).
#[pyclass(name = "LockWait", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PyLockWait {
    Nowait,
    SkipLocked,
}

#[pymethods]
impl PyLockWait {
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "lock waits cannot be tested as Python booleans",
        ))
    }
}

/// The names `OF` locks: each FROM item by the name it answers to in the
/// statement, its alias or an unaliased table's own name, unqualified as
/// PostgreSQL requires there.
fn locked_names(of: &Bound<'_, PyAny>) -> PyResult<Vec<Name>> {
    let items = of
        .extract::<Vec<Bound<'_, PyAny>>>()
        .map_err(|_| PyTypeError::new_err("a lock's of is a list of Table or FromItem"))?;
    items
        .iter()
        .map(|item| Ok(from_item(item)?.qualifier().clone()))
        .collect()
}

// [spec:pgorm:req:bindings.row-locks]
/// Set `select`'s locking clause, replacing any it had.
pub(super) fn lock(
    select: &mut SelectStatement,
    strength: &PyLockStrength,
    of: Option<&Bound<'_, PyAny>>,
    wait: Option<&PyLockWait>,
) -> PyResult<()> {
    let strength = match strength {
        PyLockStrength::Update => LockType::Update,
        PyLockStrength::NoKeyUpdate => LockType::NoKeyUpdate,
        PyLockStrength::Share => LockType::Share,
        PyLockStrength::KeyShare => LockType::KeyShare,
    };
    let tables = of.map(locked_names).transpose()?.unwrap_or_default();
    match wait {
        Some(PyLockWait::Nowait) => {
            select.lock_with_tables_behavior(strength, tables, LockBehavior::Nowait)
        }
        Some(PyLockWait::SkipLocked) => {
            select.lock_with_tables_behavior(strength, tables, LockBehavior::SkipLocked)
        }
        None => select.lock_with_tables(strength, tables),
    };
    Ok(())
}
