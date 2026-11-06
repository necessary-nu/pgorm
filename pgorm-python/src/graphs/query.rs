use super::{
    backend::{Change, Query},
    cursor::PyEntityGraphCursor,
};
use crate::panics::future_into_py;
use crate::{
    entities::{
        reads::NativeRowDecoder,
        reads_py::{CursorState, PyEntityPaginator, compiled, counted, stream},
    },
    errors::{ConstructionError, InternalError},
    execution::Executor,
    expressions::{Compiled, OrderBy, PyCondition, PyExpr},
    transactions::work::{Output, Read, Work},
};
use pgorm::pgorm_query::{Expr, IntoCondition, Name};
use pyo3::types::PyList;
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyInt, PyTuple},
};

#[derive(Clone, Debug)]
#[pyclass(name = "EntityGraphQuery", module = "pgorm", frozen, from_py_object)]
pub struct PyEntityGraphQuery {
    pub(crate) inner: Query,
}

#[pymethods]
impl PyEntityGraphQuery {
    #[getter]
    fn graph_name(&self) -> &str {
        &self.inner.info().name
    }

    fn col(&self, source: &Bound<'_, PyAny>, column: &str) -> PyResult<PyExpr> {
        if !source.is_exact_instance_of::<PyInt>() {
            return Err(PyTypeError::new_err(
                "graph source index requires an integer",
            ));
        }
        let source: usize = source
            .extract()
            .map_err(|_| ConstructionError::new_err("invalid graph source index"))?;
        let descriptor = self
            .inner
            .info()
            .bindings
            .sources
            .get(source)
            .ok_or_else(|| {
                ConstructionError::new_err("source is outside the registered graph shape")
            })?;
        descriptor.entity.column(column)?;
        let qualifier = if source == 0 {
            &descriptor.entity.table
        } else {
            &self.inner.aliases()[source - 1]
        };
        Ok(PyExpr::from_rust(
            Expr::col((Name::runtime(qualifier), Name::runtime(column))).into(),
        ))
    }

    fn r#where(&self, predicate: &Bound<'_, PyAny>) -> PyResult<Self> {
        let condition = if let Ok(condition) = predicate.extract::<PyRef<'_, PyCondition>>() {
            condition.inner.clone()
        } else if let Ok(expr) = predicate.extract::<PyRef<'_, PyExpr>>() {
            expr.inner.clone().into_condition()
        } else {
            return Err(PyTypeError::new_err(
                "graph filters require an Expr or Condition",
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

    /// The SQL and values a terminal sends, `optional` with its `LIMIT 1`
    /// and `count` the paginator's count.
    #[pyo3(signature = (*, terminal="all"))]
    fn inspect(&self, terminal: &str) -> PyResult<Compiled> {
        let optional = match terminal {
            "all" => false,
            "optional" => true,
            "count" => {
                return compiled(
                    self.inner
                        .pages()
                        .statement(std::num::NonZeroU64::MIN, None)
                        .map_err(|error| ConstructionError::new_err(error.to_string()))?
                        .build(),
                );
            }
            _ => return Err(PyTypeError::new_err("unknown graph terminal")),
        };
        compiled(self.inner.compile(optional))
    }

    /// A keyset cursor ordered by root columns, `SelectGraph::cursor_by`,
    /// every source's primary key after them as tiebreaks.
    #[pyo3(signature = (*columns))]
    fn cursor(&self, columns: Vec<String>) -> PyResult<PyEntityGraphCursor> {
        self.cursor_on(0, columns)
    }

    /// A keyset cursor ordered by columns of decoded source `source`, the
    /// first slot being 1, `SelectGraph::cursor_by_on::<I, _>`: the source's
    /// own key, then the root's and the other slots', as tiebreaks.
    #[pyo3(signature = (source, *columns))]
    fn cursor_on(&self, source: usize, columns: Vec<String>) -> PyResult<PyEntityGraphCursor> {
        let entity = &self
            .inner
            .info()
            .bindings
            .sources
            .get(source)
            .ok_or_else(|| ConstructionError::new_err(format!("the graph has no source {source}")))?
            .entity;
        for column in &columns {
            entity.column(column)?;
        }
        Ok(PyEntityGraphCursor {
            state: CursorState::new(self.inner.cursor(source, &columns)?),
        })
    }

    /// How many rows the graph reads, by `PaginatorTrait::count`.
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

    /// Pages of `page_size` rows of the join, `PaginatorTrait::paginate`.
    fn paginate(&self, page_size: &Bound<'_, PyAny>) -> PyResult<PyEntityPaginator> {
        PyEntityPaginator::new(self.inner.pages(), page_size)
    }

    /// Each root with its one `Opt` slot's models, `all_grouped`.
    fn all_grouped<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let read = self.inner.grouped()?;
        let target = Executor::extract(py, connection)?;
        future_into_py(py, async move {
            let Output::Grouped(groups) = target.run(Work::Read(read)).await? else {
                return Err(InternalError::new_err("unexpected grouped graph result"));
            };
            Python::attach(|py| {
                let groups = groups
                    .into_iter()
                    .map(|(root, slots)| {
                        root.validate(py)?;
                        for slot in &slots {
                            slot.validate(py)?;
                        }
                        Ok((Py::new(py, root)?, PyList::new(py, slots)?.unbind()))
                    })
                    .collect::<PyResult<Vec<_>>>()?;
                Ok(PyList::new(py, groups)?.into_any().unbind())
            })
        })
    }

    /// The rows one per pull, `SelectGraph::stream`'s decode of each as it
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
            compiled(self.inner.compile(false))?,
            Some(decoder),
        )
    }

    fn all<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        super::io::read(py, connection, self.inner.clone(), super::io::Read::All)
    }

    fn optional<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        super::io::read(
            py,
            connection,
            self.inner.clone(),
            super::io::Read::Optional,
        )
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "graph queries cannot be tested as Python booleans",
        ))
    }
}
