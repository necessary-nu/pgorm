use pgorm::pgorm_query::{Asterisk, FromItem, JoinType, Query, SelectStatement, UnionType};
use pyo3::{prelude::*, types::PyTuple};

use super::{
    common,
    from_item::{PyFromItem, from_item},
    grouping::{self, Grouped},
    lock::{PyLockStrength, PyLockWait, lock},
    window::window_definition,
};
use crate::{errors::ConstructionError, expressions::Compiled, identifiers::Identifier};

/// Join kinds which require an ON condition; CROSS JOIN has its own method.
#[pyclass(name = "Join", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Join {
    Inner,
    Left,
    Right,
    Full,
}

#[pymethods]
impl Join {
    // [spec:pgorm:req:python.truth-testing+4]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "join kinds cannot be tested as Python booleans",
        ))
    }
}

impl Join {
    fn rust_join(&self) -> JoinType {
        match self {
            Self::Inner => JoinType::InnerJoin,
            Self::Left => JoinType::LeftJoin,
            Self::Right => JoinType::RightJoin,
            Self::Full => JoinType::FullOuterJoin,
        }
    }
}

// [spec:pgorm:req:bindings.select+4]
// [spec:pgorm:req:bindings.statements]
/// An immutable runtime SELECT backed by a real Rust SelectStatement.
#[pyclass(name = "Select", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PySelect {
    pub inner: SelectStatement,
}

impl PySelect {
    // [spec:pgorm:req:bindings.select+4]
    // [spec:pgorm:req:bindings.windows]
    /// Refuse a tie limit, and a WINDOW clause or a reference to one, that
    /// PostgreSQL would refuse, with Rust's `check_clauses`, which rendering
    /// leaves to its caller.
    fn check(&self) -> PyResult<()> {
        self.inner
            .check_clauses()
            .map_err(|error| ConstructionError::new_err(error.to_string()))
    }

    // [spec:pgorm:req:bindings.select+4]
    /// The statement as another builder reads it — a CTE body, a subquery.
    /// Nothing can order it or declare its windows once it is embedded, so
    /// both are checked as it is read rather than left to the server.
    pub(crate) fn embedded(&self) -> PyResult<SelectStatement> {
        self.check()?;
        Ok(self.inner.clone())
    }

    // [spec:pgorm:req:bindings.select+4]
    /// Append `other` as a parenthesised set-operation arm, as Rust's `union`
    /// does for every operator. The receiver's ORDER BY, row limit and OFFSET
    /// render after the last arm, so they order and bound the combined rows
    /// whenever they were set; the arm keeps its own inside its parentheses.
    /// A chain combines in call order: the renderer parenthesises what came
    /// before an `INTERSECT` that follows a `UNION` or `EXCEPT`, which
    /// PostgreSQL's precedence would otherwise bind to the arm before it alone.
    fn combined(&self, operation: UnionType, other: &PySelect) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner.union(operation, other.embedded()?);
        Ok(next)
    }
}

#[pymethods]
impl PySelect {
    #[new]
    #[pyo3(signature = (*items))]
    pub fn new(items: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let mut inner = Query::select();
        if items.is_empty() {
            inner.column(Asterisk);
        } else {
            common::project(&mut inner, items)?;
        }
        Ok(Self { inner })
    }

    #[pyo3(signature = (*items))]
    fn select(&self, items: &Bound<'_, PyTuple>) -> PyResult<Self> {
        if items.is_empty() {
            return Err(ConstructionError::new_err(
                "select requires at least one projection",
            ));
        }
        let mut next = self.clone();
        next.inner.clear_selects();
        common::project(&mut next.inner, items)?;
        Ok(next)
    }

    /// Add a FROM item, a `Table` or a `FromItem`; a second is comma-joined
    /// and may read the columns of the items before it when it is a function
    /// such as `JSON_TABLE`, which PostgreSQL makes implicitly LATERAL.
    fn from_(&self, item: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner.from(from_item(item)?);
        Ok(next)
    }

    fn r#where(&self, predicate: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner.cond_where(common::condition(predicate)?);
        Ok(next)
    }

    // [spec:pgorm:req:bindings.from-subqueries]
    /// `lateral` reads a subquery `LATERAL`, so it may read the columns of
    /// the items before it.
    #[pyo3(signature = (item, on, *, kind=Join::Inner, lateral=false))]
    fn join(
        &self,
        item: &Bound<'_, PyAny>,
        on: &Bound<'_, PyAny>,
        kind: Join,
        lateral: bool,
    ) -> PyResult<Self> {
        let (item, on) = (from_item(item)?, common::condition(on)?);
        let mut next = self.clone();
        match (lateral, item) {
            (false, item) => next.inner.join(kind.rust_join(), item, on),
            (true, FromItem::SubQuery(query, alias)) => {
                next.inner.join_lateral(kind.rust_join(), query, alias, on)
            }
            (true, _) => {
                return Err(ConstructionError::new_err(
                    "lateral applies to a subquery, a Select made a FROM item with as_(alias)",
                ));
            }
        };
        Ok(next)
    }

    fn cross_join(&self, item: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner.cross_join(from_item(item)?);
        Ok(next)
    }

    // [spec:pgorm:req:bindings.select+4]
    /// Append to the GROUP BY list, in call order: an expression is a set of
    /// one, and a `GroupingElement` is `()`, a parenthesised set, `ROLLUP`,
    /// `CUBE` or `GROUPING SETS`.
    #[pyo3(signature = (*items))]
    fn group_by(&self, items: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let mut next = self.clone();
        for item in items.iter() {
            match grouping::element(&item)? {
                Grouped::Expr(expr) => next.inner.add_group_by([expr]),
                Grouped::Element(element) => next.inner.group_by_element(element),
            };
        }
        Ok(next)
    }

    // [spec:pgorm:req:bindings.select+4]
    /// `GROUP BY DISTINCT`: each grouping set the list names more than once
    /// is grouped once. It quantifies the whole list, and renders nothing
    /// while there is no GROUP BY.
    fn group_by_distinct(&self) -> Self {
        let mut next = self.clone();
        next.inner.group_by_distinct();
        next
    }

    fn having(&self, predicate: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner.cond_having(common::condition(predicate)?);
        Ok(next)
    }

    #[pyo3(signature = (*orderings))]
    fn order_by(&self, orderings: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let mut next = self.clone();
        common::order(&mut next.inner, orderings)?;
        Ok(next)
    }

    /// `LIMIT n`, replacing a tie limit; `None` clears either.
    fn limit(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        if value.is_none() {
            next.inner.reset_limit();
        } else {
            next.inner.limit(common::unsigned(value)?);
        }
        Ok(next)
    }

    // [spec:pgorm:req:bindings.select+4]
    /// `FETCH FIRST n ROWS WITH TIES`: the first `count` rows and every row
    /// tied with the last of them on the ORDER BY, in `limit`'s slot. It
    /// needs an ORDER BY, which may come later in the chain, so the check is
    /// made when the statement is inspected or executed.
    fn limit_with_ties(&self, count: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner.limit_with_ties(common::unsigned(count)?);
        Ok(next)
    }

    fn offset(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        if value.is_none() {
            next.inner.reset_offset();
        } else {
            next.inner.offset(common::unsigned(value)?);
        }
        Ok(next)
    }

    fn distinct(&self) -> Self {
        let mut next = self.clone();
        next.inner.distinct();
        next
    }

    // [spec:pgorm:req:bindings.windows]
    /// Append a named window to the WINDOW clause, `name AS (..)`, which an
    /// item's `over(name)` reads: a `Window`, or an `ExtendedWindow` built on
    /// an entry appended before it. A clause PostgreSQL refuses is refused
    /// when the statement is inspected, executed or read into another.
    fn window(&self, name: &Bound<'_, PyAny>, window: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner
            .window(Identifier::new(name)?.name(), window_definition(window)?);
        Ok(next)
    }

    /// Prefix a WITH clause; the last call wins. A MERGE body there is read
    /// through its RETURNING list.
    fn with_(&self, clause: PyRef<'_, super::with::PyWith>) -> Self {
        let mut next = self.clone();
        next.inner.with(clause.inner.clone());
        next
    }

    // [spec:pgorm:req:bindings.from-subqueries]
    /// This query as a FROM item, `(SELECT ..) AS "alias"`, read wherever a
    /// FROM item is. It is final once read, so its tie limit and windows are
    /// checked here.
    fn as_(&self, alias: &Bound<'_, PyAny>) -> PyResult<PyFromItem> {
        let alias = Identifier::new(alias)?.name();
        Ok(PyFromItem {
            inner: FromItem::SubQuery(self.embedded()?, alias),
        })
    }

    // [spec:pgorm:req:bindings.row-locks]
    /// Lock the rows read, replacing any lock set before: `of` names the FROM
    /// items locked, every one the statement reads when it is absent.
    #[pyo3(signature = (strength, *, of=None, wait=None))]
    fn lock(
        &self,
        strength: PyLockStrength,
        of: Option<&Bound<'_, PyAny>>,
        wait: Option<PyLockWait>,
    ) -> PyResult<Self> {
        let mut next = self.clone();
        lock(&mut next.inner, &strength, of, wait.as_ref())?;
        Ok(next)
    }

    /// `UNION`: the rows of either query, each distinct row once.
    fn union(&self, other: PyRef<'_, PySelect>) -> PyResult<Self> {
        self.combined(UnionType::Distinct, &other)
    }

    /// `UNION ALL`: the rows of both queries, duplicates kept.
    fn union_all(&self, other: PyRef<'_, PySelect>) -> PyResult<Self> {
        self.combined(UnionType::All, &other)
    }

    /// `INTERSECT`: the distinct rows both queries return.
    fn intersect(&self, other: PyRef<'_, PySelect>) -> PyResult<Self> {
        self.combined(UnionType::Intersect, &other)
    }

    /// `INTERSECT ALL`: a row as many times as the query returning it less
    /// often does.
    fn intersect_all(&self, other: PyRef<'_, PySelect>) -> PyResult<Self> {
        self.combined(UnionType::IntersectAll, &other)
    }

    /// `EXCEPT`, named with a trailing underscore because `except` is a
    /// Python keyword: the distinct rows of this query that `other` lacks.
    fn except_(&self, other: PyRef<'_, PySelect>) -> PyResult<Self> {
        self.combined(UnionType::Except, &other)
    }

    /// `EXCEPT ALL`: a row as many times as this query returns it beyond the
    /// times `other` does.
    fn except_all(&self, other: PyRef<'_, PySelect>) -> PyResult<Self> {
        self.combined(UnionType::ExceptAll, &other)
    }

    // [spec:pgorm:req:bindings.statements]
    // [spec:pgorm:req:bindings.select+4]
    /// Every execution path renders through this method, so the tie-limit
    /// and window checks Rust leaves to the caller are made here, before any
    /// SQL exists.
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
        "Select(...)"
    }
}
