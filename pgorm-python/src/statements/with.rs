//! Common table expressions: a WITH clause of one or more, and the recursive
//! form, which holds exactly one with its optional SEARCH and CYCLE clauses;
//! and the two versions of a written row a RETURNING list reads.

use pgorm::pgorm_query::{
    AnyWithClause, Asterisk, CommonTableExpression, Cycle, Expr, RecursiveWithClause, ReturningRow,
    Search, SearchOrder, WithClause,
};
use pyo3::{exceptions::PyTypeError, prelude::*};

use super::{
    insert::PyInsert,
    merge::PyMerge,
    select::PySelect,
    write::{PyDelete, PyUpdate},
};
use crate::{
    errors::ConstructionError,
    expressions::{PyExpr, coerce},
    identifiers::Identifier,
};

// [spec:pgorm:req:bindings.with+1]
// [spec:pgorm:req:bindings.write-ctes]
// [spec:pgorm:req:bindings.cte-options]
/// One common table expression. Its body is a SELECT, or an INSERT, UPDATE,
/// DELETE or MERGE, whose rows are those its RETURNING list yields. A body is
/// final once read, so a SELECT's tie limit and windows, and a write that
/// could not run, are refused here.
fn expression(
    name: &Bound<'_, PyAny>,
    query: &Bound<'_, PyAny>,
    columns: Option<&Bound<'_, PyAny>>,
    materialized: Option<bool>,
) -> PyResult<CommonTableExpression> {
    let name = Identifier::new(name)?.name();
    let mut expression = if let Ok(select) = query.extract::<PyRef<'_, PySelect>>() {
        CommonTableExpression::new(name, select.embedded()?)
    } else if let Ok(insert) = query.extract::<PyRef<'_, PyInsert>>() {
        insert.check()?;
        CommonTableExpression::new(name, insert.inner.clone())
    } else if let Ok(update) = query.extract::<PyRef<'_, PyUpdate>>() {
        update.check()?;
        CommonTableExpression::new(name, update.inner.clone())
    } else if let Ok(delete) = query.extract::<PyRef<'_, PyDelete>>() {
        delete.check()?;
        CommonTableExpression::new(name, delete.inner.clone())
    } else if let Ok(merge) = query.extract::<PyRef<'_, PyMerge>>() {
        CommonTableExpression::new(name, merge.inner.clone())
    } else {
        return Err(PyTypeError::new_err(
            "a common table expression's body is a Select, Insert, Update, Delete or Merge",
        ));
    };
    if let Some(columns) = columns {
        let columns = columns
            .extract::<Vec<Bound<'_, PyAny>>>()
            .map_err(|_| PyTypeError::new_err("a common table expression's columns are a list"))?;
        for column in columns {
            expression.column(Identifier::new(&column)?.name());
        }
    }
    if let Some(materialized) = materialized {
        expression.materialized(materialized);
    }
    Ok(expression)
}

/// `WITH name AS (query), ..`: one common table expression or more, in order,
/// or `WITH RECURSIVE` and its one.
#[pyclass(name = "With", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyWith {
    pub(super) inner: AnyWithClause,
}

#[pymethods]
impl PyWith {
    #[new]
    #[pyo3(signature = (name, query, *, columns=None, materialized=None))]
    fn new(
        name: &Bound<'_, PyAny>,
        query: &Bound<'_, PyAny>,
        columns: Option<&Bound<'_, PyAny>>,
        materialized: Option<bool>,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: WithClause::new(expression(name, query, columns, materialized)?).into(),
        })
    }

    // [spec:pgorm:req:bindings.cte-options]
    /// One more common table expression, which may read the ones before it;
    /// a recursive clause holds exactly one.
    #[pyo3(signature = (name, query, *, columns=None, materialized=None))]
    fn cte(
        &self,
        name: &Bound<'_, PyAny>,
        query: &Bound<'_, PyAny>,
        columns: Option<&Bound<'_, PyAny>>,
        materialized: Option<bool>,
    ) -> PyResult<Self> {
        let AnyWithClause::Plain(clause) = &self.inner else {
            return Err(ConstructionError::new_err(
                "a recursive WITH holds exactly one common table expression",
            ));
        };
        let mut clause = clause.clone();
        clause.cte(expression(name, query, columns, materialized)?);
        Ok(Self {
            inner: clause.into(),
        })
    }

    // [spec:pgorm:req:bindings.cte-options]
    /// `WITH RECURSIVE`: one common table expression whose `Select` body may
    /// read itself, with SEARCH and CYCLE when they are given.
    #[staticmethod]
    #[pyo3(signature = (name, query, *, columns=None, materialized=None, search=None, cycle=None))]
    fn recursive(
        name: &Bound<'_, PyAny>,
        query: &Bound<'_, PyAny>,
        columns: Option<&Bound<'_, PyAny>>,
        materialized: Option<bool>,
        search: Option<PyRef<'_, PySearch>>,
        cycle: Option<PyRef<'_, PyCycle>>,
    ) -> PyResult<Self> {
        if !query.is_instance_of::<PySelect>() {
            return Err(PyTypeError::new_err(
                "a recursive common table expression's body is a Select",
            ));
        }
        let mut clause = RecursiveWithClause::new(expression(name, query, columns, materialized)?);
        if let Some(search) = search {
            clause.search(search.inner.clone());
        }
        if let Some(cycle) = cycle {
            clause.cycle(cycle.inner.clone());
        }
        Ok(Self {
            inner: clause.into(),
        })
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            AnyWithClause::Plain(clause) => format!("With(ctes={})", clause.ctes().count()),
            AnyWithClause::Recursive(_) => "With(recursive)".to_owned(),
        }
    }

    // [spec:pgorm:req:python.truth-testing+5]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "WITH clauses cannot be tested as Python booleans",
        ))
    }
}

impl PyWith {
    // [spec:pgorm:req:bindings.cte-options]
    /// The clause a MERGE takes: never a recursive one, which PostgreSQL
    /// refuses before a MERGE.
    pub(super) fn plain(&self) -> PyResult<WithClause> {
        match &self.inner {
            AnyWithClause::Plain(clause) => Ok(clause.clone()),
            AnyWithClause::Recursive(_) => Err(ConstructionError::new_err(
                "PostgreSQL takes no WITH RECURSIVE before a MERGE",
            )),
        }
    }
}

/// The order a recursive query's SEARCH walks its rows in.
#[pyclass(name = "SearchOrder", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PySearchOrder {
    Breadth,
    Depth,
}

#[pymethods]
impl PySearchOrder {
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "search orders cannot be tested as Python booleans",
        ))
    }
}

// [spec:pgorm:req:bindings.cte-options]
/// `SEARCH BREADTH | DEPTH FIRST BY by SET set`: the column `set` orders a
/// recursive query's rows breadth or depth first by `by`.
#[pyclass(name = "Search", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PySearch {
    inner: Search,
}

#[pymethods]
impl PySearch {
    #[new]
    fn new(order: PySearchOrder, by: &Bound<'_, PyAny>, set: &Bound<'_, PyAny>) -> PyResult<Self> {
        let order = match order {
            PySearchOrder::Breadth => SearchOrder::BREADTH,
            PySearchOrder::Depth => SearchOrder::DEPTH,
        };
        Ok(Self {
            inner: Search::new(order, coerce(by)?.inner, Identifier::new(set)?.name()),
        })
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SEARCH clauses cannot be tested as Python booleans",
        ))
    }
}

// [spec:pgorm:req:bindings.cte-options]
/// `CYCLE by SET set USING using`: the column `set` marks a row whose `by`
/// the path column `using` already holds, and the recursion stops there.
#[pyclass(name = "Cycle", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyCycle {
    inner: Cycle,
}

#[pymethods]
impl PyCycle {
    #[new]
    fn new(
        by: &Bound<'_, PyAny>,
        set: &Bound<'_, PyAny>,
        using: &Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: Cycle::new(
                coerce(by)?.inner,
                Identifier::new(set)?.name(),
                Identifier::new(using)?.name(),
            ),
        })
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "CYCLE clauses cannot be tested as Python booleans",
        ))
    }
}

/// The row as it was before the write (`Old`) or as the statement left it
/// (`New`), read in a RETURNING list. With `returning(old_as=..)` or
/// `new_as=..` the version answers only to its new name, read as a table.
#[pyclass(name = "ReturningRow", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PyReturningRow {
    Old,
    New,
}

impl PyReturningRow {
    fn rust(&self) -> ReturningRow {
        match self {
            Self::Old => ReturningRow::Old,
            Self::New => ReturningRow::New,
        }
    }
}

#[pymethods]
impl PyReturningRow {
    fn col(&self, name: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
        let name = Identifier::new(name)?.name();
        Ok(PyExpr::from_rust(Expr::col((self.rust(), name)).into()))
    }

    fn star(&self) -> PyExpr {
        PyExpr::from_rust(Expr::col((self.rust(), Asterisk)).into())
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "row versions cannot be tested as Python booleans",
        ))
    }
}
