//! The Python classes of a registered read's pages, keyset cursor, partial
//! projection and relations, and the terminals that run them.

use std::{
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use pgorm::{
    CursorPlan, CursorWindow, boundary_columns,
    pgorm_query::{ValueTuple, Values},
};
use pyo3::{
    IntoPyObjectExt,
    exceptions::{PyStopAsyncIteration, PyTypeError},
    prelude::*,
    types::{PyList, PyTuple},
};
use pyo3_async_runtimes::tokio::future_into_py;

use super::{
    query::bound,
    reads::{KeyColumn, Keyset, NativeRowDecoder, Pages, page_size, row_object},
};
use crate::{
    errors::{ConstructionError, InternalError},
    execution::Executor,
    expressions::Compiled,
    transactions::work::{Output, Read, Work},
};

/// The statement, refused when it binds more values than PostgreSQL takes.
pub(crate) fn compiled((sql, values): (String, Values)) -> PyResult<Compiled> {
    if values.0.len() > 65535 {
        return Err(ConstructionError::new_err(
            "PostgreSQL supports at most 65535 query parameters",
        ));
    }
    Ok(Compiled { sql, values })
}

/// Run `read`, whose output is rows, and give them as Python reads them: a
/// list, or with `single` the first row or `None`.
pub(crate) fn rows<'py>(
    py: Python<'py>,
    connection: &Bound<'_, PyAny>,
    read: Read,
    single: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let target = Executor::extract(py, connection)?;
    future_into_py(py, async move {
        let Output::Graph(rows) = target.run(Work::Read(read)).await? else {
            return Err(InternalError::new_err("unexpected registered read result"));
        };
        Python::attach(|py| {
            let mut rows = rows
                .into_iter()
                .map(|row| row_object(py, row))
                .collect::<PyResult<Vec<_>>>()?;
            if single {
                return Ok(if rows.is_empty() {
                    py.None()
                } else {
                    rows.swap_remove(0)
                });
            }
            Ok(PyList::new(py, rows)?.into_any().unbind())
        })
    })
}

/// Run `read`, whose output is a count.
pub(crate) fn counted<'py>(
    py: Python<'py>,
    connection: &Bound<'_, PyAny>,
    read: Read,
) -> PyResult<Bound<'py, PyAny>> {
    let target = Executor::extract(py, connection)?;
    future_into_py(py, async move {
        let Output::Count(count) = target.run(Work::Read(read)).await? else {
            return Err(InternalError::new_err("unexpected registered count result"));
        };
        Ok(count)
    })
}

/// A stream of `statement`'s rows, each decoded by `decoder`, opened by the
/// package's stream helper on a pool's connection of its own or on a
/// connection, which the stream holds until it ends.
// [spec:pgorm:req:bindings.streams]
pub(crate) fn stream<'py>(
    py: Python<'py>,
    connection: &Bound<'py, PyAny>,
    statement: Compiled,
    decoder: Option<NativeRowDecoder>,
) -> PyResult<Bound<'py, PyAny>> {
    let statement = Bound::new(py, statement)?.into_any();
    let decoder = match decoder {
        Some(decoder) => Bound::new(py, decoder)?.into_any(),
        None => py.None().into_bound(py),
    };
    py.import("pgorm._registered.streams")?
        .getattr("open_stream")?
        .call1((connection, statement, decoder))
}

/// A registered read's pages, Rust's `Paginator`, which holds no database
/// and no current page: each terminal takes the database it runs on.
// [spec:pgorm:req:bindings.entity-reads+4]
#[pyclass(
    name = "EntityPaginator",
    module = "pgorm",
    frozen,
    generic,
    skip_from_py_object
)]
#[derive(Clone, Debug)]
pub struct PyEntityPaginator {
    pub(crate) pages: Arc<Pages>,
    pub(crate) size: NonZeroU64,
}

impl PyEntityPaginator {
    pub(crate) fn new(pages: Arc<Pages>, size: &Bound<'_, PyAny>) -> PyResult<Self> {
        let size =
            bound(Some(size))?.ok_or_else(|| PyTypeError::new_err("a page size is an int"))?;
        Ok(Self {
            pages,
            size: page_size(size)?,
        })
    }

    fn fetch(&self, page: u64) -> Read {
        let (pages, size) = (self.pages.clone(), self.size);
        Box::new(move |db| {
            Box::pin(async move { pages.fetch(db, size, page).await.map(Output::Graph) })
        })
    }

    /// Run `Paginator::num_items_and_pages`, answering with what `answer`
    /// takes of the rows and the pages.
    fn numbers<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
        answer: fn(u64, u64) -> Option<(u64, u64)>,
        single: fn(u64, u64) -> u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let target = Executor::extract(py, connection)?;
        let (pages, size) = (self.pages.clone(), self.size);
        let read: Read = Box::new(move |db| {
            Box::pin(async move {
                let counted = pages.count(db, size).await?;
                Ok(Output::Counted(
                    counted.number_of_items,
                    counted.number_of_pages,
                ))
            })
        });
        future_into_py(py, async move {
            let Output::Counted(items, pages) = target.run(Work::Read(read)).await? else {
                return Err(InternalError::new_err("unexpected paginator count"));
            };
            Python::attach(|py| match answer(items, pages) {
                Some(pair) => pair.into_py_any(py),
                None => single(items, pages).into_py_any(py),
            })
        })
    }
}

#[pymethods]
impl PyEntityPaginator {
    /// Page `page`, numbered from zero, by `Paginator::fetch_page`.
    fn fetch_page<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
        page: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let page =
            bound(Some(page))?.ok_or_else(|| PyTypeError::new_err("a page number is an int"))?;
        self.pages
            .statement(self.size, Some(page))
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        rows(py, connection, self.fetch(page), false)
    }

    fn num_items<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.numbers(py, connection, |_, _| None, |items, _| items)
    }

    fn num_pages<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.numbers(py, connection, |_, _| None, |_, pages| pages)
    }

    fn num_items_and_pages<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.numbers(
            py,
            connection,
            |items, pages| Some((items, pages)),
            |items, _| items,
        )
    }

    /// Each page in turn from the first, ending at the first empty one, as
    /// Rust's `fetch_and_next` loop and `into_stream` read them.
    fn pages(&self, connection: Py<PyAny>) -> PyEntityPages {
        PyEntityPages {
            paginator: self.clone(),
            connection,
            next: Arc::new(AtomicU64::new(0)),
            done: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The statement reading page `page`, or counting the rows with
    /// `"count"`.
    fn inspect(&self, page: &Bound<'_, PyAny>) -> PyResult<Compiled> {
        if let Ok(terminal) = page.extract::<String>() {
            if terminal != "count" {
                return Err(PyTypeError::new_err(
                    "a paginator inspects a page, or its \"count\"",
                ));
            }
            return compiled(
                self.pages
                    .statement(self.size, None)
                    .map_err(|error| ConstructionError::new_err(error.to_string()))?
                    .build(),
            );
        }
        let page =
            bound(Some(page))?.ok_or_else(|| PyTypeError::new_err("a page number is an int"))?;
        compiled(
            self.pages
                .statement(self.size, Some(page))
                .map_err(|error| ConstructionError::new_err(error.to_string()))?
                .build(),
        )
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity paginators cannot be tested as Python booleans",
        ))
    }
}

/// The pages of a paginator, read one per `__anext__`.
#[pyclass(name = "_EntityPages", module = "pgorm._native", frozen)]
#[derive(Debug)]
pub(crate) struct PyEntityPages {
    paginator: PyEntityPaginator,
    connection: Py<PyAny>,
    next: Arc<AtomicU64>,
    done: Arc<AtomicBool>,
}

#[pymethods]
impl PyEntityPages {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        if self.done.load(Ordering::SeqCst) {
            return Err(PyStopAsyncIteration::new_err(()));
        }
        let page = self.next.fetch_add(1, Ordering::SeqCst);
        let target = Executor::extract(py, self.connection.bind(py))?;
        let read = self.paginator.fetch(page);
        let done = self.done.clone();
        future_into_py(py, async move {
            let Output::Graph(rows) = target.run(Work::Read(read)).await? else {
                return Err(InternalError::new_err("unexpected page result"));
            };
            if rows.is_empty() {
                done.store(true, Ordering::SeqCst);
                return Err(PyStopAsyncIteration::new_err(()));
            }
            Python::attach(|py| {
                let rows = rows
                    .into_iter()
                    .map(|row| row_object(py, row))
                    .collect::<PyResult<Vec<_>>>()?;
                Ok(PyList::new(py, rows)?.into_any().unbind())
            })
        })
    }
}

/// A keyset cursor's state: what it orders by and its plan, each method
/// giving a new state.
#[derive(Clone, Debug)]
pub(crate) struct CursorState {
    pub(crate) keyset: Arc<Keyset>,
    pub(crate) plan: CursorPlan,
}

impl CursorState {
    pub(crate) fn new(keyset: Arc<Keyset>) -> Self {
        Self {
            keyset,
            plan: CursorPlan::default(),
        }
    }

    /// A boundary of the order columns' values, or with `whole` of the
    /// order columns' or the whole key's, each converted through its column.
    pub(crate) fn bound(
        &self,
        before: bool,
        values: &Bound<'_, PyTuple>,
        whole: bool,
    ) -> PyResult<Self> {
        let columns: &[KeyColumn] =
            boundary_columns(&self.keyset.order, &self.keyset.key, values.len(), whole)
                .map_err(|arity| ConstructionError::new_err(arity.to_string()))?;
        let tuple = columns
            .iter()
            .zip(values)
            .map(|(column, value)| {
                Ok(column
                    .entity
                    .column(&column.column)?
                    .input
                    .coerce(&value)?
                    .inner)
            })
            .collect::<PyResult<Vec<_>>>()?
            .into_iter()
            .collect::<ValueTuple>();
        let mut next = self.clone();
        if before {
            next.plan.before = Some(tuple);
        } else {
            next.plan.after = Some(tuple);
        }
        Ok(next)
    }

    pub(crate) fn window(&self, last: bool, count: &Bound<'_, PyAny>) -> PyResult<Self> {
        let count = bound(Some(count))?
            .ok_or_else(|| PyTypeError::new_err("cursor window requires a row count"))?;
        let mut next = self.clone();
        next.plan.window = Some(if last {
            CursorWindow::Last(count)
        } else {
            CursorWindow::First(count)
        });
        Ok(next)
    }

    pub(crate) fn direction(&self, descending: bool) -> Self {
        let mut next = self.clone();
        next.plan.descending = descending;
        next
    }

    pub(crate) fn inspect(&self) -> PyResult<Compiled> {
        compiled(
            self.keyset
                .cursor
                .statement(&self.plan)
                .map_err(|error| ConstructionError::new_err(error.to_string()))?
                .build(),
        )
    }

    pub(crate) fn all<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.inspect()?;
        let (keyset, plan) = (self.keyset.clone(), self.plan.clone());
        let read: Read = Box::new(move |db| {
            Box::pin(async move { keyset.cursor.all(db, plan).await.map(Output::Graph) })
        });
        rows(py, connection, read, false)
    }
}

/// A registered `Select<E>`'s keyset cursor, `Select::cursor_by`.
// [spec:pgorm:req:bindings.entity-reads+4]
#[pyclass(name = "EntityCursor", module = "pgorm", frozen, skip_from_py_object)]
#[derive(Clone, Debug)]
pub struct PyEntityCursor {
    pub(crate) state: CursorState,
}

#[pymethods]
impl PyEntityCursor {
    #[pyo3(signature = (*values))]
    fn before(&self, values: &Bound<'_, PyTuple>) -> PyResult<Self> {
        Ok(Self {
            state: self.state.bound(true, values, false)?,
        })
    }
    #[pyo3(signature = (*values))]
    fn after(&self, values: &Bound<'_, PyTuple>) -> PyResult<Self> {
        Ok(Self {
            state: self.state.bound(false, values, false)?,
        })
    }
    fn first(&self, count: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            state: self.state.window(false, count)?,
        })
    }
    fn last(&self, count: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            state: self.state.window(true, count)?,
        })
    }
    fn asc(&self) -> Self {
        Self {
            state: self.state.direction(false),
        }
    }
    fn desc(&self) -> Self {
        Self {
            state: self.state.direction(true),
        }
    }
    /// The statement `all` sends.
    fn inspect(&self) -> PyResult<Compiled> {
        self.state.inspect()
    }
    fn all<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.state.all(py, connection)
    }
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity cursors cannot be tested as Python booleans",
        ))
    }
}
