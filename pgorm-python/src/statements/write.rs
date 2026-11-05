use pgorm::pgorm_query::{DeleteStatement, Query, UpdateStatement};
use pyo3::{prelude::*, types::PyTuple};

use super::{common, from_item::from_item, table::PyTable, with::PyWith};
use crate::{
    errors::ConstructionError,
    expressions::{Compiled, coerce},
    identifiers::Identifier,
};

// [spec:pgorm:req:bindings.writes+2]
/// A runtime UPDATE with explicit predicates or all-rows intent.
#[pyclass(name = "Update", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyUpdate {
    pub inner: UpdateStatement,
    guarded: bool,
}

impl PyUpdate {
    // [spec:pgorm:req:bindings.writes+2]
    /// Refuse an UPDATE that cannot run, whether it is inspected, run or read
    /// into a WITH clause.
    pub(crate) fn check(&self) -> PyResult<()> {
        if !self.guarded {
            return Err(ConstructionError::new_err(
                "update requires where or explicit all_rows",
            ));
        }
        if self.inner.get_values().is_empty() {
            return Err(ConstructionError::new_err(
                "update requires at least one assignment",
            ));
        }
        Ok(())
    }
}

#[pymethods]
impl PyUpdate {
    #[new]
    fn new(table: PyRef<'_, PyTable>) -> Self {
        Self {
            inner: Query::update().table(table.inner.clone()).to_owned(),
            guarded: false,
        }
    }

    fn set(&self, column: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let name = Identifier::new(column)?;
        if self
            .inner
            .get_values()
            .iter()
            .any(|(column, _)| column.to_string() == name.name)
        {
            return Err(ConstructionError::new_err("duplicate update assignment"));
        }
        let mut next = self.clone();
        next.inner.value(name.name(), coerce(value)?.inner);
        Ok(next)
    }

    fn r#where(&self, predicate: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner.cond_where(common::condition(predicate)?);
        next.guarded = true;
        Ok(next)
    }

    fn all_rows(&self) -> Self {
        let mut next = self.clone();
        next.guarded = true;
        next
    }

    #[pyo3(signature = (*items, old_as=None, new_as=None))]
    fn returning(
        &self,
        items: &Bound<'_, PyTuple>,
        old_as: Option<&Bound<'_, PyAny>>,
        new_as: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner
            .returning(common::returning(items, old_as, new_as)?);
        Ok(next)
    }

    // [spec:pgorm:req:bindings.write-sources]
    /// One more FROM item the UPDATE reads, a `Table` or a `FromItem`.
    fn from_(&self, item: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner.from(from_item(item)?);
        Ok(next)
    }

    // [spec:pgorm:req:bindings.write-ctes]
    /// Prefix a WITH clause; the last call wins.
    fn with_(&self, clause: PyRef<'_, PyWith>) -> Self {
        let mut next = self.clone();
        next.inner.with(clause.inner.clone());
        next
    }

    pub fn inspect(&self) -> PyResult<Compiled> {
        self.check()?;
        common::compiled(self.inner.build())
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL statements cannot be tested as Python booleans",
        ))
    }
    fn __repr__(&self) -> &'static str {
        "Update(...)"
    }
}

// [spec:pgorm:req:bindings.writes+2]
/// A runtime DELETE with explicit predicates or all-rows intent.
#[pyclass(name = "Delete", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyDelete {
    pub inner: DeleteStatement,
    guarded: bool,
}

impl PyDelete {
    // [spec:pgorm:req:bindings.writes+2]
    /// Refuse a DELETE with neither a WHERE nor `all_rows()`, whether it is
    /// inspected, run or read into a WITH clause.
    pub(crate) fn check(&self) -> PyResult<()> {
        if !self.guarded {
            return Err(ConstructionError::new_err(
                "delete requires where or explicit all_rows",
            ));
        }
        Ok(())
    }
}

#[pymethods]
impl PyDelete {
    #[new]
    fn new(table: PyRef<'_, PyTable>) -> Self {
        Self {
            inner: Query::delete().from_table(table.inner.clone()).to_owned(),
            guarded: false,
        }
    }

    fn r#where(&self, predicate: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner.cond_where(common::condition(predicate)?);
        next.guarded = true;
        Ok(next)
    }

    fn all_rows(&self) -> Self {
        let mut next = self.clone();
        next.guarded = true;
        next
    }

    #[pyo3(signature = (*items, old_as=None, new_as=None))]
    fn returning(
        &self,
        items: &Bound<'_, PyTuple>,
        old_as: Option<&Bound<'_, PyAny>>,
        new_as: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner
            .returning(common::returning(items, old_as, new_as)?);
        Ok(next)
    }

    // [spec:pgorm:req:bindings.write-sources]
    /// One more FROM item the DELETE reads through `USING`, a `Table` or a
    /// `FromItem`.
    fn using(&self, item: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner.using(from_item(item)?);
        Ok(next)
    }

    // [spec:pgorm:req:bindings.write-ctes]
    /// Prefix a WITH clause; the last call wins.
    fn with_(&self, clause: PyRef<'_, PyWith>) -> Self {
        let mut next = self.clone();
        next.inner.with(clause.inner.clone());
        next
    }

    pub fn inspect(&self) -> PyResult<Compiled> {
        self.check()?;
        common::compiled(self.inner.build())
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL statements cannot be tested as Python booleans",
        ))
    }
    fn __repr__(&self) -> &'static str {
        "Delete(...)"
    }
}
