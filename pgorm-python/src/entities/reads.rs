//! The reads a registered `Select<E>` and `SelectGraph<E, S>` have beside
//! `all` and `one_opt`: their pages, fetched and counted by pgorm's own
//! `Paginator`; their keyset cursor, pgorm's `Cursor` with its key type
//! erased behind the boundaries' `_with` forms, which take any arity and
//! leave it to the cursor to check; and their rows as a stream, each decoded
//! by the read's own selector as it arrives.

use std::{fmt, num::NonZeroU64, sync::Arc};

use futures_util::future::BoxFuture;
use pgorm::{
    Cursor, Error, PaginatorTrait, QueryResult, QueryTrait, SelectorTrait, count_statement,
    page_statement,
    pgorm_query::{SelectStatement, ValueTuple, Values},
};
use pyo3::{prelude::*, types::PyTuple};

use super::{PyEntityModel, metadata::EntityInfo};
use crate::{errors::DecodeError, execution::Database, graphs::backend::Row};

/// A read's pages: page `n` at a page size, and the count of its rows, each
/// as pgorm's `Paginator` reads them.
pub(crate) trait Pages: fmt::Debug + Send + Sync {
    /// The statement reading page `page`, or counting the rows with `None`.
    fn statement(&self, size: NonZeroU64, page: Option<u64>) -> Result<(String, Values), Error>;
    /// `Paginator::fetch_page`.
    fn fetch<'a>(
        &'a self,
        db: Database<'a>,
        size: NonZeroU64,
        page: u64,
    ) -> BoxFuture<'a, Result<Vec<Row>, Error>>;
    /// `Paginator::num_items_and_pages`: the rows, and the pages of `size`
    /// they fill.
    fn count<'a>(
        &'a self,
        db: Database<'a>,
        size: NonZeroU64,
    ) -> BoxFuture<'a, Result<(u64, u64), Error>>;
}

/// A read `Q` whose pages pgorm's `PaginatorTrait` reads through selector
/// `S`, and the decode of its items into rows: one implementation for an
/// entity's `Select` and a graph's `SelectGraph`.
pub(crate) struct Paged<Q, S: SelectorTrait> {
    pub(crate) query: Q,
    pub(crate) decode: Arc<dyn Fn(S::Item) -> Row + Send + Sync>,
}

impl<Q, S: SelectorTrait> fmt::Debug for Paged<Q, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Paged").finish_non_exhaustive()
    }
}

// [spec:pgorm:req:bindings.entity-reads+4]
// [spec:pgorm:req:bindings.entity-graphs+4]
impl<Q, S> Pages for Paged<Q, S>
where
    Q: QueryTrait<QueryStatement = SelectStatement> + Clone + Send + Sync + 'static,
    for<'db, 'c> Q: PaginatorTrait<'db, Database<'c>, Selector = S>,
    S: SelectorTrait + Send + Sync + 'static,
    S::Item: Send,
{
    fn statement(&self, size: NonZeroU64, page: Option<u64>) -> Result<(String, Values), Error> {
        let query = QueryTrait::as_query(&self.query);
        Ok(match page {
            Some(page) => page_statement(query, size, page)?.build(),
            None => count_statement(query)?.build(),
        })
    }

    fn fetch<'a>(
        &'a self,
        db: Database<'a>,
        size: NonZeroU64,
        page: u64,
    ) -> BoxFuture<'a, Result<Vec<Row>, Error>> {
        Box::pin(async move {
            let items = self
                .query
                .clone()
                .paginate(&db, size)
                .fetch_page(page)
                .await?;
            Ok(items.into_iter().map(|item| (self.decode)(item)).collect())
        })
    }

    fn count<'a>(
        &'a self,
        db: Database<'a>,
        size: NonZeroU64,
    ) -> BoxFuture<'a, Result<(u64, u64), Error>> {
        Box::pin(async move {
            let counted = self
                .query
                .clone()
                .paginate(&db, size)
                .num_items_and_pages()
                .await?;
            Ok((counted.number_of_items, counted.number_of_pages))
        })
    }
}

/// A keyset cursor's boundaries, window and direction, applied to pgorm's
/// `Cursor` as it runs.
#[derive(Clone, Debug, Default)]
pub(crate) struct Plan {
    pub(crate) before: Option<ValueTuple>,
    pub(crate) after: Option<ValueTuple>,
    pub(crate) descending: bool,
    pub(crate) window: Option<(bool, u64)>,
}

/// A column a cursor's boundary value converts through: its entity, and its
/// name there.
#[derive(Clone, Debug)]
pub(crate) struct KeyColumn {
    pub(crate) entity: Arc<EntityInfo>,
    pub(crate) column: String,
}

/// A registered read's keyset cursor: its order columns, the whole key a
/// `_with` boundary names, the statement a plan composes and its rows.
pub(crate) trait Keyset: fmt::Debug + Send + Sync {
    fn order(&self) -> &[KeyColumn];
    fn key(&self) -> &[KeyColumn];
    fn statement(&self, plan: &Plan) -> Result<(String, Values), Error>;
    fn all<'a>(&'a self, db: Database<'a>, plan: Plan) -> BoxFuture<'a, Result<Vec<Row>, Error>>;
}

/// pgorm's cursor over a selector `S`, made afresh for each plan since a
/// cursor of a selector that is not `Clone` is not, and the decode of its
/// items into rows.
pub(crate) struct Keyed<S: SelectorTrait, K> {
    pub(crate) cursor: Box<dyn Fn() -> Cursor<S, K> + Send + Sync>,
    pub(crate) decode: Arc<dyn Fn(S::Item) -> Row + Send + Sync>,
    pub(crate) order: Vec<KeyColumn>,
    pub(crate) key: Vec<KeyColumn>,
}

impl<S: SelectorTrait, K> fmt::Debug for Keyed<S, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keyed")
            .field("order", &self.order)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl<S: SelectorTrait, K> Keyed<S, K> {
    fn planned(&self, plan: &Plan) -> Cursor<S, K> {
        let mut cursor = (self.cursor)();
        if let Some(values) = plan.before.clone() {
            cursor.before_with(values);
        }
        if let Some(values) = plan.after.clone() {
            cursor.after_with(values);
        }
        if plan.descending {
            cursor.desc();
        } else {
            cursor.asc();
        }
        match plan.window {
            Some((true, rows)) => {
                cursor.last(rows);
            }
            Some((false, rows)) => {
                cursor.first(rows);
            }
            None => {}
        }
        cursor
    }
}

// [spec:pgorm:req:bindings.entity-reads+4]
// [spec:pgorm:req:bindings.entity-graphs+4]
impl<S, K> Keyset for Keyed<S, K>
where
    S: SelectorTrait + Send + Sync + 'static,
    S::Item: Send,
    K: Send + Sync + 'static,
{
    fn order(&self) -> &[KeyColumn] {
        &self.order
    }

    fn key(&self) -> &[KeyColumn] {
        &self.key
    }

    fn statement(&self, plan: &Plan) -> Result<(String, Values), Error> {
        Ok(self.planned(plan).statement()?.build())
    }

    fn all<'a>(&'a self, db: Database<'a>, plan: Plan) -> BoxFuture<'a, Result<Vec<Row>, Error>> {
        Box::pin(async move {
            let items = self.planned(&plan).all(&db).await?;
            Ok(items.into_iter().map(|item| (self.decode)(item)).collect())
        })
    }
}

/// The decode of one row of a registered read's result into its sources'
/// models, as its selector's `from_raw_query_result` reads it.
pub(crate) type Decode = Arc<dyn Fn(QueryResult) -> Result<Row, Error> + Send + Sync>;

/// A row as Python receives it: a read of one source's model alone, `None`
/// for an absent optional one, and otherwise a tuple of each source's.
pub(crate) fn row_object(py: Python<'_>, row: Row) -> PyResult<Py<PyAny>> {
    for model in row.iter().flatten() {
        model.validate(py)?;
    }
    if row.len() == 1 {
        return Ok(match row.into_iter().next().flatten() {
            Some(model) => Py::new(py, model)?.into_any(),
            None => py.None(),
        });
    }
    Ok(PyTuple::new(py, row)?.into_any().unbind())
}

/// The decode a registered read's stream runs on each row, handed to a
/// connection's `stream` beside the read's statement.
// [spec:pgorm:req:bindings.streams]
#[pyclass(name = "_RowDecoder", module = "pgorm._native", frozen)]
pub(crate) struct NativeRowDecoder {
    pub(crate) decode: Decode,
}

impl fmt::Debug for NativeRowDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeRowDecoder").finish_non_exhaustive()
    }
}

impl NativeRowDecoder {
    pub(crate) fn row_decode(&self) -> crate::results::RowDecode {
        let decode = self.decode.clone();
        Arc::new(move |row| {
            let row = decode(QueryResult::from(row))
                .map_err(|error| DecodeError::new_err(error.to_string()))?;
            Python::attach(|py| row_object(py, row))
        })
    }
}

/// The model a registered entity's row decodes as, in a read of that entity
/// alone.
pub(crate) fn one_source(model: PyEntityModel) -> Row {
    vec![Some(model)]
}

/// A page size as pgorm's paginator takes one, refused below 1.
pub(crate) fn page_size(size: u64) -> PyResult<NonZeroU64> {
    NonZeroU64::new(size)
        .ok_or_else(|| crate::errors::ConstructionError::new_err("a page size is at least 1"))
}
