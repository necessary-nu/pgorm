use pgorm::pgorm_query::IntoCondition;
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyInt, PyTuple},
};

use super::{
    backend::{Change, Select, Terminal},
    reads::NativeRowDecoder,
    reads_py::{CursorState, PyEntityCursor, PyEntityPaginator, compiled, counted, stream},
    selection::{PyEntityRelation, PyEntitySelection},
};
use crate::{
    errors::ConstructionError,
    expressions::{Compiled, OrderBy, PyCondition, PyExpr},
    statements::Join,
    transactions::work::{Output, Read},
};

#[derive(Clone, Debug)]
#[pyclass(name = "EntityQuery", module = "pgorm", frozen, from_py_object)]
pub struct PyEntityQuery {
    pub(crate) inner: Select,
}

pub(crate) fn bound(value: Option<&Bound<'_, PyAny>>) -> PyResult<Option<u64>> {
    let Some(value) = value.filter(|v| !v.is_none()) else {
        return Ok(None);
    };
    if !value.is_exact_instance_of::<PyInt>() {
        return Err(PyTypeError::new_err("query bounds require an integer"));
    }
    let number: i64 = value
        .extract()
        .map_err(|_| ConstructionError::new_err("query bound exceeds i64"))?;
    u64::try_from(number)
        .map(Some)
        .map_err(|_| ConstructionError::new_err("query bounds must be nonnegative"))
}

#[pymethods]
impl PyEntityQuery {
    #[getter]
    fn entity_name(&self) -> &str {
        &self.inner.info().name
    }

    fn r#where(&self, predicate: &Bound<'_, PyAny>) -> PyResult<Self> {
        let condition = if let Ok(condition) = predicate.extract::<PyRef<'_, PyCondition>>() {
            condition.inner.clone()
        } else if let Ok(expr) = predicate.extract::<PyRef<'_, PyExpr>>() {
            expr.inner.clone().into_condition()
        } else {
            return Err(PyTypeError::new_err(
                "entity filters require an Expr or Condition",
            ));
        };
        Ok(Self {
            inner: self.inner.change(Change::Filter(condition)),
        })
    }

    #[pyo3(signature = (*ordering))]
    fn order_by(&self, ordering: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let mut inner = self.inner.clone();
        for order in ordering {
            let order = order
                .extract::<PyRef<'_, OrderBy>>()
                .map_err(|_| PyTypeError::new_err("order_by requires typed OrderBy values"))?;
            inner = inner.change(Change::Order(
                order.expr.inner.clone(),
                order.direction.rust_order(),
                order.nulls.as_ref().map(|n| n.rust_nulls()),
            ));
        }
        Ok(Self { inner })
    }

    #[pyo3(signature = (value=None))]
    fn limit(&self, value: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.change(Change::Limit(bound(value)?)),
        })
    }
    #[pyo3(signature = (value=None))]
    fn offset(&self, value: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.change(Change::Offset(bound(value)?)),
        })
    }

    /// The SQL and values a terminal sends: `one` and `optional` with the
    /// limit Rust's terminals narrow to, `count` the paginator's count.
    #[pyo3(signature = (*, terminal="all"))]
    fn inspect(&self, terminal: &str) -> PyResult<Compiled> {
        let terminal = match terminal {
            "all" => Terminal::All,
            "one" => Terminal::One,
            "optional" => Terminal::Optional,
            "count" => {
                return compiled(
                    self.inner
                        .pages()
                        .statement(std::num::NonZeroU64::MIN, None)
                        .map_err(|error| ConstructionError::new_err(error.to_string()))?
                        .build(),
                );
            }
            _ => return Err(PyTypeError::new_err("unknown entity terminal")),
        };
        compiled(self.inner.compile(terminal))
    }

    /// The relation's far end joined, `QuerySelect::join`: its rows stay this
    /// entity's models, and the joined table's columns go in conditions and
    /// orderings through its own entity's `col`.
    #[pyo3(signature = (relation, *, kind=Join::Inner))]
    fn join(&self, relation: PyRef<'_, PyEntityRelation>, kind: Join) -> PyResult<Self> {
        relation
            .entity
            .relation(&relation.name)
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        Ok(Self {
            inner: self.inner.change(Change::Join(
                kind.rust_join(),
                relation.entity.clone(),
                relation.name.clone(),
            )),
        })
    }

    /// The query projected onto `columns`, `select_only` and `columns`, read
    /// as records of those columns rather than as models.
    #[pyo3(signature = (*columns))]
    fn select(&self, columns: &Bound<'_, PyTuple>) -> PyResult<PyEntitySelection> {
        if columns.is_empty() {
            return Err(ConstructionError::new_err(
                "a selection names at least one column",
            ));
        }
        let columns = columns
            .iter()
            .map(|column| super::column::name(&column, self.inner.info()))
            .collect::<PyResult<Vec<_>>>()?;
        let selection = PyEntitySelection {
            select: self.inner.clone(),
            columns,
        };
        selection.statement(Terminal::All)?;
        Ok(selection)
    }

    /// How many rows the query reads, its limit, offset and ordering aside,
    /// by `PaginatorTrait::count`.
    fn count<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inspect("count")?;
        let pages = self.inner.pages();
        let read: Read = Box::new(move |db| {
            Box::pin(async move {
                let counted = pages.count(db, std::num::NonZeroU64::MIN).await?;
                Ok(Output::Count(counted.number_of_items))
            })
        });
        counted(py, connection, read)
    }

    /// Pages of `page_size` rows, by `PaginatorTrait::paginate`.
    fn paginate(&self, page_size: &Bound<'_, PyAny>) -> PyResult<PyEntityPaginator> {
        PyEntityPaginator::new(self.inner.pages(), page_size)
    }

    /// A keyset cursor ordered by `columns`, `Select::cursor_by`; its
    /// ordering replaces the query's.
    #[pyo3(signature = (*columns))]
    fn cursor(&self, columns: &Bound<'_, PyTuple>) -> PyResult<PyEntityCursor> {
        let columns = columns
            .iter()
            .map(|column| super::column::name(&column, self.inner.info()))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(PyEntityCursor {
            state: CursorState::new(
                self.inner
                    .cursor(&columns)
                    .map_err(|error| ConstructionError::new_err(error.to_string()))?,
            ),
        })
    }

    /// The models one per pull, `Select::stream`'s decode of each row as it
    /// arrives, over a connection the stream holds until it ends.
    fn stream<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let decoder = NativeRowDecoder {
            decode: self.inner.decode(),
        };
        stream(
            py,
            connection,
            compiled(self.inner.compile(Terminal::All))?,
            Some(decoder),
        )
    }

    fn all<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        super::io::select(py, connection, self.inner.clone(), Terminal::All)
    }
    fn one<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        super::io::select(py, connection, self.inner.clone(), Terminal::One)
    }
    fn optional<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        super::io::select(py, connection, self.inner.clone(), Terminal::Optional)
    }
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity queries cannot be tested as Python booleans",
        ))
    }
    fn __repr__(&self) -> String {
        format!("EntityQuery({:?})", self.entity_name())
    }
}
