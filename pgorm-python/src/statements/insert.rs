use std::collections::HashSet;

use pgorm::pgorm_query::{InsertStatement, Query};
use pyo3::{prelude::*, types::PyTuple};

use super::{
    common, conflict, merge_action::PyOverriding, select::PySelect, table::PyTable, with::PyWith,
};
use crate::{
    errors::ConstructionError,
    expressions::{Compiled, coerce},
    identifiers::Identifier,
};

/// Where an INSERT's rows come from: none yet, `values` rows, one `Select`,
/// or one row of defaults; each excludes the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rows {
    None,
    Values(usize),
    Select,
    Defaults,
}

// [spec:pgorm:req:bindings.writes+2]
/// A runtime INSERT with explicit columns, row arity and default-row state.
#[pyclass(name = "Insert", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyInsert {
    pub inner: InsertStatement,
    columns: Vec<String>,
    rows: Rows,
}

impl PyInsert {
    // [spec:pgorm:req:bindings.writes+2]
    /// Refuse an INSERT with no rows, whether it is inspected, run or read
    /// into a WITH clause.
    pub(crate) fn check(&self) -> PyResult<()> {
        if self.rows == Rows::None {
            return Err(ConstructionError::new_err(
                "insert requires rows, a select or explicit default_values",
            ));
        }
        Ok(())
    }
}

#[pymethods]
impl PyInsert {
    #[new]
    fn new(table: PyRef<'_, PyTable>) -> Self {
        Self {
            inner: Query::insert().into_table(table.inner.clone()).to_owned(),
            columns: Vec::new(),
            rows: Rows::None,
        }
    }

    // [spec:pgorm:req:bindings.writes+2]
    /// The distinct columns the rows fill, named once, before any row.
    #[pyo3(signature = (*columns))]
    fn columns(&self, columns: &Bound<'_, PyTuple>) -> PyResult<Self> {
        if self.rows != Rows::None || !self.columns.is_empty() {
            return Err(ConstructionError::new_err(
                "an insert names its columns once, before any row",
            ));
        }
        let mut names = HashSet::new();
        let columns = columns
            .iter()
            .map(|column| {
                let column = Identifier::new(&column)?;
                if !names.insert(column.name.clone()) {
                    return Err(ConstructionError::new_err("duplicate insert column"));
                }
                Ok(column)
            })
            .collect::<PyResult<Vec<_>>>()?;
        if columns.is_empty() {
            return Err(ConstructionError::new_err(
                "use default_values for an insert without columns",
            ));
        }
        let mut next = self.clone();
        next.inner.columns(columns.iter().map(Identifier::name));
        next.columns = columns.into_iter().map(|column| column.name).collect();
        Ok(next)
    }

    #[pyo3(signature = (*values))]
    fn values(&self, values: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let written = match self.rows {
            Rows::None => 0,
            Rows::Values(written) => written,
            Rows::Select | Rows::Defaults => {
                return Err(ConstructionError::new_err(
                    "values mix with neither select nor default_values",
                ));
            }
        };
        if self.columns.is_empty() {
            return Err(ConstructionError::new_err(
                "choose insert columns before values",
            ));
        }
        let values = values
            .iter()
            .map(|value| coerce(&value).map(|expr| expr.inner))
            .collect::<PyResult<Vec<_>>>()?;
        let mut next = self.clone();
        next.inner
            .values(values)
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        next.rows = Rows::Values(written + 1);
        Ok(next)
    }

    // [spec:pgorm:req:bindings.write-sources]
    /// The rows `query` yields, as many columns as the INSERT names; they are
    /// its only rows. The query is final once read, so its tie limit and
    /// windows are checked here.
    fn select(&self, query: PyRef<'_, PySelect>) -> PyResult<Self> {
        if self.columns.is_empty() || self.rows != Rows::None {
            return Err(ConstructionError::new_err(
                "select follows columns, and is an insert's only rows",
            ));
        }
        let mut next = self.clone();
        next.inner
            .select_from(query.embedded()?)
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        next.rows = Rows::Select;
        Ok(next)
    }

    fn default_values(&self) -> PyResult<Self> {
        if self.rows != Rows::None || !self.columns.is_empty() {
            return Err(ConstructionError::new_err(
                "default values cannot be mixed with insert columns or rows",
            ));
        }
        let mut next = self.clone();
        next.inner.or_default_values();
        next.rows = Rows::Defaults;
        Ok(next)
    }

    // [spec:pgorm:req:bindings.write-sources]
    /// `OVERRIDING SYSTEM VALUE` or `OVERRIDING USER VALUE`, for identity
    /// columns; the last call wins.
    fn overriding(&self, overriding: PyOverriding) -> Self {
        let mut next = self.clone();
        next.inner.overriding(overriding.rust());
        next
    }

    fn on_conflict(&self, action: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner.on_conflict(conflict::clause(action)?);
        Ok(next)
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
    fn __repr__(&self) -> String {
        format!("Insert(rows={:?})", self.rows)
    }
}
