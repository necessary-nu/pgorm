//! The actions a MERGE arm takes, each made by `MergeAction`'s factories: an
//! update, a delete or nothing for a target row, and an insert, an insert of
//! defaults or nothing for a source row no target row matched. An update and
//! an insert are never empty, each column arriving with its value. An action
//! is checked against the arm's kind of row as the arm reads it.

use pgorm::pgorm_query::{MatchedAction, MergeInsert, MergeUpdate, NotMatchedAction, Overriding};
use pyo3::{exceptions::PyTypeError, prelude::*};

use crate::{errors::ConstructionError, expressions::coerce, identifiers::Identifier};

/// The actions that carry nothing but their keyword.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Keyword {
    Delete,
    DoNothing,
    InsertDefaults,
}

// [spec:pgorm:req:bindings.merge+2]
/// What an arm does with its row: `DELETE`, `DO NOTHING` or `INSERT DEFAULT
/// VALUES` itself, and through its factories an update or an insert.
#[pyclass(
    name = "MergeAction",
    module = "pgorm",
    frozen,
    generic,
    from_py_object
)]
#[derive(Clone, Debug)]
pub struct PyMergeAction {
    keyword: Keyword,
}

#[pymethods]
impl PyMergeAction {
    /// `UPDATE SET column = value`, for a target row; `set` adds assignments.
    #[staticmethod]
    fn update(column: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<PyMergeUpdate> {
        let column = Identifier::new(column)?.name();
        Ok(PyMergeUpdate {
            inner: MergeUpdate::value(column, coerce(value)?.inner),
        })
    }

    /// `INSERT (column) VALUES (value)`, for a source row; `set` adds columns.
    #[staticmethod]
    fn insert(column: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<PyMergeInsert> {
        let column = Identifier::new(column)?.name();
        Ok(PyMergeInsert {
            inner: MergeInsert::value(column, coerce(value)?.inner),
        })
    }

    /// `DELETE`, for a target row.
    #[staticmethod]
    fn delete() -> Self {
        Self {
            keyword: Keyword::Delete,
        }
    }

    /// `DO NOTHING`, for any row; a row left alone is not returned.
    #[staticmethod]
    fn do_nothing() -> Self {
        Self {
            keyword: Keyword::DoNothing,
        }
    }

    /// `INSERT DEFAULT VALUES`, for a source row.
    #[staticmethod]
    fn insert_defaults() -> Self {
        Self {
            keyword: Keyword::InsertDefaults,
        }
    }

    fn __repr__(&self) -> &'static str {
        match self.keyword {
            Keyword::Delete => "MergeAction.delete()",
            Keyword::DoNothing => "MergeAction.do_nothing()",
            Keyword::InsertDefaults => "MergeAction.insert_defaults()",
        }
    }

    // [spec:pgorm:req:python.truth-testing+5]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "MERGE actions cannot be tested as Python booleans",
        ))
    }
}

/// What an inserted row's identity columns do with a supplied value.
#[pyclass(name = "Overriding", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PyOverriding {
    SystemValue,
    UserValue,
}

#[pymethods]
impl PyOverriding {
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "OVERRIDING clauses cannot be tested as Python booleans",
        ))
    }
}

impl PyOverriding {
    pub(super) fn rust(&self) -> Overriding {
        match self {
            Self::SystemValue => Overriding::SystemValue,
            Self::UserValue => Overriding::UserValue,
        }
    }
}

/// `UPDATE SET column = value, ..` for a target row, made by
/// `MergeAction.update`.
#[pyclass(name = "MergeUpdate", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyMergeUpdate {
    inner: MergeUpdate,
}

#[pymethods]
impl PyMergeUpdate {
    fn set(&self, column: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let column = Identifier::new(column)?.name();
        let value = coerce(value)?.inner;
        Ok(Self {
            inner: self.inner.clone().and_value(column, value),
        })
    }

    fn __repr__(&self) -> &'static str {
        "MergeUpdate(...)"
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "MERGE actions cannot be tested as Python booleans",
        ))
    }
}

/// `INSERT (column, ..) VALUES (value, ..)` for a source row, made by
/// `MergeAction.insert`.
#[pyclass(name = "MergeInsert", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyMergeInsert {
    inner: MergeInsert,
}

#[pymethods]
impl PyMergeInsert {
    fn set(&self, column: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let column = Identifier::new(column)?.name();
        let value = coerce(value)?.inner;
        Ok(Self {
            inner: self.inner.clone().and_value(column, value),
        })
    }

    /// `OVERRIDING SYSTEM VALUE` or `OVERRIDING USER VALUE`; the last call wins.
    fn overriding(&self, overriding: PyOverriding) -> Self {
        Self {
            inner: self.inner.clone().overriding(overriding.rust()),
        }
    }

    fn __repr__(&self) -> &'static str {
        "MergeInsert(...)"
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "MERGE actions cannot be tested as Python booleans",
        ))
    }
}

/// An action as an arm reads it, before it is matched to the arm's kind of
/// row.
enum Action {
    Update(MergeUpdate),
    Insert(MergeInsert),
    Keyword(Keyword),
}

impl Action {
    fn read(action: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(update) = action.extract::<PyRef<'_, PyMergeUpdate>>() {
            Ok(Self::Update(update.inner.clone()))
        } else if let Ok(insert) = action.extract::<PyRef<'_, PyMergeInsert>>() {
            Ok(Self::Insert(insert.inner.clone()))
        } else if let Ok(action) = action.extract::<PyRef<'_, PyMergeAction>>() {
            Ok(Self::Keyword(action.keyword))
        } else {
            Err(PyTypeError::new_err(
                "an arm takes a MergeAction: MergeAction.update(..), .insert(..), .delete(), \
                 .do_nothing() or .insert_defaults()",
            ))
        }
    }

    fn describe(&self) -> &'static str {
        match self {
            Self::Update(_) => "an update",
            Self::Insert(_) => "an insert",
            Self::Keyword(Keyword::Delete) => "a delete",
            Self::Keyword(Keyword::DoNothing) => "doing nothing",
            Self::Keyword(Keyword::InsertDefaults) => "an insert of defaults",
        }
    }
}

// [spec:pgorm:req:bindings.merge+2]
/// The action of an arm that takes a target row: an update, a delete or
/// nothing. An insert has no target row to take, so it is refused here as the
/// Rust types refuse it.
pub(super) fn matched(action: &Bound<'_, PyAny>) -> PyResult<MatchedAction> {
    match Action::read(action)? {
        Action::Update(update) => Ok(update.into()),
        Action::Keyword(Keyword::Delete) => Ok(MatchedAction::Delete),
        Action::Keyword(Keyword::DoNothing) => Ok(MatchedAction::DoNothing),
        other => Err(ConstructionError::new_err(format!(
            "an arm on a target row updates, deletes or does nothing, not {}",
            other.describe()
        ))),
    }
}

/// The action of an arm that takes a source row no target row matched: an
/// insert or nothing, never an update or a delete.
pub(super) fn not_matched(action: &Bound<'_, PyAny>) -> PyResult<NotMatchedAction> {
    match Action::read(action)? {
        Action::Insert(insert) => Ok(insert.into()),
        Action::Keyword(Keyword::InsertDefaults) => Ok(NotMatchedAction::InsertDefaultValues),
        Action::Keyword(Keyword::DoNothing) => Ok(NotMatchedAction::DoNothing),
        other => Err(ConstructionError::new_err(format!(
            "a NOT MATCHED arm inserts or does nothing, not {}",
            other.describe()
        ))),
    }
}
