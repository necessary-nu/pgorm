//! The reads a registered `Select<E>` and `SelectGraph<E, S>` have beside
//! `all` and `one_opt`: their pages, fetched and counted by pgorm's own
//! `Paginator`; their keyset cursor, pgorm's `Cursor` with its key type
//! erased behind the boundaries' `_with` forms, which take any arity and
//! leave it to the cursor to check; and their rows as a stream, each decoded
//! by the read's own selector as it arrives.

use pgorm_bindings::Metrics;
use std::{fmt, num::NonZeroU64, sync::Arc};

use neon::{prelude::*, types::Finalize};
use pgorm::{
    CursorPlan, CursorWindow, ErasedCursor, ErasedPages, Error, QueryResult, boundary_columns,
    pgorm_query::ValueTuple,
};

use super::{
    adapter::Model,
    convert::{GraphRows, column_value},
    info::EntityInfo,
};
use crate::{
    connect::job::{Job, JobHandle, Output},
    errors::Failure,
    values::read,
};

/// A row of a registered read: each source's model, `None` for an absent
/// optional slot.
pub(crate) type Row = Vec<Option<Model>>;

/// A registered read's pages: pgorm's erased paginator, and the entities its
/// rows' sources decode as.
#[derive(Debug)]
pub(crate) struct Pages {
    pub(crate) pages: Box<dyn ErasedPages<Row, Metrics>>,
    pub(crate) sources: Vec<Arc<EntityInfo>>,
}

/// A column a cursor's boundary value converts through: its entity, and its
/// name there.
#[derive(Clone, Debug)]
pub(crate) struct KeyColumn {
    pub(crate) entity: Arc<EntityInfo>,
    pub(crate) column: String,
}

/// A registered read's keyset cursor: pgorm's erased cursor, its order
/// columns, the whole key a `_with` boundary names, and the entities its
/// rows' sources decode as.
#[derive(Debug)]
pub(crate) struct Keyset {
    pub(crate) cursor: Arc<dyn ErasedCursor<Row, Metrics>>,
    pub(crate) order: Vec<KeyColumn>,
    pub(crate) key: Vec<KeyColumn>,
    pub(crate) sources: Vec<Arc<EntityInfo>>,
}

/// The decode of one row of a registered read's result into its sources'
/// models, as its selector's `from_raw_query_result` reads it.
pub(crate) type Decode = Arc<dyn Fn(QueryResult) -> Result<Row, Error> + Send + Sync>;

/// The decode a registered read's stream runs on each row, handed to
/// `connectionStream` beside the read's statement.
pub(crate) struct DecoderHandle {
    pub(crate) decode: Decode,
    pub(crate) sources: Vec<Arc<EntityInfo>>,
}

impl fmt::Debug for DecoderHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecoderHandle").finish_non_exhaustive()
    }
}

impl Finalize for DecoderHandle {}

impl DecoderHandle {
    /// The stream's decode of a row into the rows JavaScript reads.
    // [spec:pgorm:req:bindings.streams+1]
    pub(crate) fn row_decode(&self) -> crate::connect::stream::RowDecode {
        let (decode, sources) = (self.decode.clone(), self.sources.clone());
        Arc::new(move |row| {
            let row = decode(QueryResult::from(row))
                .map_err(|error| Failure::Decode(error.to_string()))?;
            Ok(Box::new(GraphRows(sources.clone(), vec![row])) as Output)
        })
    }
}

/// A page size as pgorm's paginator takes one, refused below 1.
pub(crate) fn page_size(cx: &mut FunctionContext, index: usize) -> NeonResult<NonZeroU64> {
    let size = whole(cx, index, "a page size")?;
    match NonZeroU64::new(size) {
        Some(size) => Ok(size),
        None => read::refuse(cx, "a page size is at least 1"),
    }
}

/// The non-negative integer at argument `index`.
pub(crate) fn whole(cx: &mut FunctionContext, index: usize, what: &str) -> NeonResult<u64> {
    let number = cx.argument::<JsNumber>(index)?.value(cx);
    if number.fract() != 0.0 || !(0.0..=9_007_199_254_740_991.0).contains(&number) {
        return read::refuse(cx, format!("{what} is a non-negative integer"));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(number as u64)
}

/// A keyset cursor as JavaScript holds it: what it orders by and its plan.
#[derive(Debug)]
pub(crate) struct CursorHandle(pub(crate) Arc<Keyset>, pub(crate) CursorPlan);

impl Finalize for CursorHandle {}

/// A read's pages as JavaScript holds them.
#[derive(Debug)]
pub(crate) struct PagesHandle(pub(crate) Arc<Pages>, pub(crate) NonZeroU64);

impl Finalize for PagesHandle {}

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("cursorBound", cursor_bound)?;
    cx.export_function("cursorWindow", cursor_window)?;
    cx.export_function("cursorDirection", cursor_direction)?;
    cx.export_function("cursorInspect", cursor_inspect)?;
    cx.export_function("cursorJob", cursor_job)?;
    cx.export_function("pagesInspect", pages_inspect)?;
    cx.export_function("pagesFetchJob", pages_fetch_job)?;
    cx.export_function("pagesCountJob", pages_count_job)?;
    Ok(())
}

fn cursor(cx: &mut FunctionContext) -> NeonResult<(Arc<Keyset>, CursorPlan)> {
    let handle = cx.argument::<JsBox<CursorHandle>>(0)?;
    Ok((handle.0.clone(), handle.1.clone()))
}

/// `cursorBound(cursor, before, values, whole)`: a boundary of the order
/// columns' values, or with `whole` of the order columns' or the whole key's,
/// each converted to its column's declared kind; any other count is refused
/// here, before pgorm's own check when the cursor composes.
// [spec:pgorm:req:bindings.entity-reads+4]
// [spec:pgorm:req:bindings.entity-graphs+4]
fn cursor_bound(mut cx: FunctionContext) -> JsResult<JsValue> {
    let (keyset, mut plan) = cursor(&mut cx)?;
    let before = cx.argument::<JsBoolean>(1)?.value(&mut cx);
    let values = cx.argument::<JsArray>(2)?.to_vec(&mut cx)?;
    let extended = cx.argument::<JsBoolean>(3)?.value(&mut cx);
    let columns = match boundary_columns(&keyset.order, &keyset.key, values.len(), extended) {
        Ok(columns) => columns,
        Err(arity) => return read::refuse(&mut cx, arity.to_string()),
    };
    let mut bound = Vec::with_capacity(values.len());
    for (column, value) in columns.iter().zip(values) {
        let info = match column.entity.column(&column.column) {
            Ok(info) => info.clone(),
            Err(failure) => return crate::errors::throw(&mut cx, failure),
        };
        bound.push(column_value(&mut cx, &info, value)?);
    }
    let bound: ValueTuple = bound.into_iter().collect();
    if before {
        plan.before = Some(bound);
    } else {
        plan.after = Some(bound);
    }
    Ok(cx.boxed(CursorHandle(keyset, plan)).upcast())
}

fn cursor_window(mut cx: FunctionContext) -> JsResult<JsValue> {
    let (keyset, mut plan) = cursor(&mut cx)?;
    let last = cx.argument::<JsBoolean>(1)?.value(&mut cx);
    let rows = whole(&mut cx, 2, "a window's rows")?;
    plan.window = Some(if last {
        CursorWindow::Last(rows)
    } else {
        CursorWindow::First(rows)
    });
    Ok(cx.boxed(CursorHandle(keyset, plan)).upcast())
}

fn cursor_direction(mut cx: FunctionContext) -> JsResult<JsValue> {
    let (keyset, mut plan) = cursor(&mut cx)?;
    plan.descending = cx.argument::<JsBoolean>(1)?.value(&mut cx);
    Ok(cx.boxed(CursorHandle(keyset, plan)).upcast())
}

/// `cursorInspect(cursor)`: the statement `all` sends, `Cursor::statement`.
fn cursor_inspect(mut cx: FunctionContext) -> JsResult<JsValue> {
    let (keyset, plan) = cursor(&mut cx)?;
    match keyset.cursor.statement(&plan) {
        Ok(statement) => super::inspected(&mut cx, statement.build()),
        Err(error) => read::refuse(&mut cx, error.to_string()),
    }
}

/// `cursorJob(cursor)`: `Cursor::all`, a last window's rows in the cursor's
/// order.
fn cursor_job(mut cx: FunctionContext) -> JsResult<JsValue> {
    let (keyset, plan) = cursor(&mut cx)?;
    if let Err(error) = keyset.cursor.statement(&plan) {
        return read::refuse(&mut cx, error.to_string());
    }
    let job: Job = Box::new(move |db, secrets| {
        Box::pin(async move {
            let rows = keyset
                .cursor
                .all(db, plan)
                .await
                .map_err(|error| crate::errors::failure(&error, secrets))?;
            Ok(Box::new(GraphRows(keyset.sources.clone(), rows)) as Output)
        })
    });
    Ok(cx.boxed(JobHandle::new(job)).upcast())
}

fn pages(cx: &mut FunctionContext) -> NeonResult<(Arc<Pages>, NonZeroU64)> {
    let handle = cx.argument::<JsBox<PagesHandle>>(0)?;
    Ok((handle.0.clone(), handle.1))
}

/// `pagesInspect(pages, page)`: the statement reading page `page`, or with
/// `null` the one counting the rows.
fn pages_inspect(mut cx: FunctionContext) -> JsResult<JsValue> {
    let (pages, size) = pages(&mut cx)?;
    let page = if cx.argument::<JsValue>(1)?.is_a::<JsNull, _>(&mut cx) {
        None
    } else {
        Some(whole(&mut cx, 1, "a page number")?)
    };
    match pages.pages.statement(size, page) {
        Ok(statement) => super::inspected(&mut cx, statement.build()),
        Err(error) => read::refuse(&mut cx, error.to_string()),
    }
}

/// `pagesFetchJob(pages, page)`: `Paginator::fetch_page`.
// [spec:pgorm:req:bindings.entity-reads+4]
fn pages_fetch_job(mut cx: FunctionContext) -> JsResult<JsValue> {
    let (pages, size) = pages(&mut cx)?;
    let page = whole(&mut cx, 1, "a page number")?;
    if let Err(error) = pages.pages.statement(size, Some(page)) {
        return read::refuse(&mut cx, error.to_string());
    }
    let job: Job = Box::new(move |db, secrets| {
        Box::pin(async move {
            let rows = pages
                .pages
                .fetch(db, size, page)
                .await
                .map_err(|error| crate::errors::failure(&error, secrets))?;
            Ok(Box::new(GraphRows(pages.sources.clone(), rows)) as Output)
        })
    });
    Ok(cx.boxed(JobHandle::new(job)).upcast())
}

/// The rows a read counts and the pages they fill, `[items, pages]`.
struct Counted(u64, u64);

impl crate::connect::job::Settled for Counted {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        let pair = JsArray::new(cx, 2);
        let items = super::convert::count(cx, self.0)?;
        let pages = super::convert::count(cx, self.1)?;
        pair.set(cx, 0, items)?;
        pair.set(cx, 1, pages)?;
        Ok(pair.upcast())
    }
}

/// `pagesCountJob(pages)`: `Paginator::num_items_and_pages`.
// [spec:pgorm:req:bindings.entity-reads+4]
fn pages_count_job(mut cx: FunctionContext) -> JsResult<JsValue> {
    let (pages, size) = pages(&mut cx)?;
    if let Err(error) = pages.pages.statement(size, None) {
        return read::refuse(&mut cx, error.to_string());
    }
    let job: Job = Box::new(move |db, secrets| {
        Box::pin(async move {
            let counted = pages
                .pages
                .count(db, size)
                .await
                .map_err(|error| crate::errors::failure(&error, secrets))?;
            Ok(Box::new(Counted(counted.number_of_items, counted.number_of_pages)) as Output)
        })
    });
    Ok(cx.boxed(JobHandle::new(job)).upcast())
}
