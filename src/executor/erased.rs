//! Pages and keyset cursors with their Rust types erased, for code that picks
//! a read at run time and holds it behind one trait object whatever its
//! selector and key types: a language binding reading an entity or a graph an
//! application registered. [`ErasedPages`] reads a [`PaginatorTrait`] read's
//! pages and [`ErasedCursor`] runs a [`Cursor`] under a [`CursorPlan`], each
//! decoding its items through a function the caller gives into rows of its
//! own type, and each running on a [`DatabaseRef`].

use std::{fmt, num::NonZeroU64, sync::Arc};

use futures::future::BoxFuture;
use pgorm_query::{SelectStatement, ValueTuple};

use super::{
    Cursor, ItemsAndPagesNumber, PaginatorTrait, SelectorTrait, count_statement,
    graph::{GraphItem, GraphRow},
    page_statement,
};
use crate::{
    DatabaseRef, EntityTrait, Error, FromQueryResult, Opt, QueryTrait, Select, SelectGraph, Slot,
    SlotAt, Slots,
    metric::{MetricsCollector, NoOpMetrics},
    pgorm_query::{Key, Name},
};

/// A read's pages at any page size, as pgorm's [`Paginator`](super::Paginator)
/// reads and counts them, its items decoded into rows `R`.
// [spec:pgorm:def:exec.paginator.erased]
pub trait ErasedPages<R, M: MetricsCollector = NoOpMetrics>: fmt::Debug + Send + Sync {
    /// The statement reading page `page` of `size` rows, or with `None` the
    /// one counting the rows, as the paginator writes them.
    fn statement(&self, size: NonZeroU64, page: Option<u64>) -> Result<SelectStatement, Error>;

    /// [`Paginator::fetch_page`](super::Paginator::fetch_page).
    fn fetch<'a>(
        &'a self,
        db: DatabaseRef<'a, M>,
        size: NonZeroU64,
        page: u64,
    ) -> BoxFuture<'a, Result<Vec<R>, Error>>;

    /// [`Paginator::num_items_and_pages`](super::Paginator::num_items_and_pages).
    fn count<'a>(
        &'a self,
        db: DatabaseRef<'a, M>,
        size: NonZeroU64,
    ) -> BoxFuture<'a, Result<ItemsAndPagesNumber, Error>>;
}

/// A read `Q` whose pages [`PaginatorTrait`] reads through its selector `S`,
/// and the decode of each item into a row `R`.
///
/// ```
/// use std::num::NonZeroU64;
/// use pgorm::{EntityTrait, ErasedPages, PagedRead, tests_cfg::cake};
///
/// let pages: Box<dyn ErasedPages<String>> =
///     Box::new(PagedRead::new(cake::Entity::find(), |cake: cake::Model| cake.name));
/// let second = pages.statement(NonZeroU64::new(10).unwrap(), Some(1)).unwrap();
/// assert!(second.to_string().ends_with("LIMIT 10 OFFSET 10"));
/// ```
// [spec:pgorm:def:exec.paginator.erased]
pub struct PagedRead<Q, S: SelectorTrait, R> {
    query: Q,
    decode: Arc<dyn Fn(S::Item) -> R + Send + Sync>,
}

impl<Q, S: SelectorTrait, R> PagedRead<Q, S, R> {
    /// The pages of `query`, each item decoded by `decode`.
    pub fn new(query: Q, decode: impl Fn(S::Item) -> R + Send + Sync + 'static) -> Self {
        Self {
            query,
            decode: Arc::new(decode),
        }
    }
}

impl<Q, S: SelectorTrait, R> fmt::Debug for PagedRead<Q, S, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PagedRead").finish_non_exhaustive()
    }
}

// [spec:pgorm:def:exec.paginator.erased]
impl<Q, S, R, M> ErasedPages<R, M> for PagedRead<Q, S, R>
where
    M: MetricsCollector,
    Q: QueryTrait<QueryStatement = SelectStatement> + Clone + Send + Sync + 'static,
    for<'db, 'c> Q: PaginatorTrait<'db, DatabaseRef<'c, M>, Selector = S>,
    S: SelectorTrait + Send + Sync + 'static,
    S::Item: Send,
    R: Send + 'static,
{
    fn statement(&self, size: NonZeroU64, page: Option<u64>) -> Result<SelectStatement, Error> {
        let query = QueryTrait::as_query(&self.query);
        match page {
            Some(page) => page_statement(query, size, page),
            None => count_statement(query),
        }
    }

    fn fetch<'a>(
        &'a self,
        db: DatabaseRef<'a, M>,
        size: NonZeroU64,
        page: u64,
    ) -> BoxFuture<'a, Result<Vec<R>, Error>> {
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
        db: DatabaseRef<'a, M>,
        size: NonZeroU64,
    ) -> BoxFuture<'a, Result<ItemsAndPagesNumber, Error>> {
        Box::pin(async move {
            self.query
                .clone()
                .paginate(&db, size)
                .num_items_and_pages()
                .await
        })
    }
}

/// Which rows of a cursor's order a [`CursorPlan`] reads: the first or the
/// last `n`, as [`Cursor::first`] and [`Cursor::last`] window them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorWindow {
    /// The first `n` rows.
    First(u64),
    /// The last `n` rows, read backwards and given in the cursor's order.
    Last(u64),
}

/// The boundaries, direction and window a cursor runs under, each applied as
/// [`Cursor::before_with`], [`Cursor::after_with`], [`Cursor::desc`] and the
/// window's method apply it, so a boundary of any arity reaches the cursor,
/// which refuses one that is not its order columns' or its whole key's.
// [spec:pgorm:def:exec.cursor.erased]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CursorPlan {
    /// Rows short of this key.
    pub before: Option<ValueTuple>,
    /// Rows past this key.
    pub after: Option<ValueTuple>,
    /// Whether the cursor reads its order descending.
    pub descending: bool,
    /// The window, or every row past and short of the boundaries.
    pub window: Option<CursorWindow>,
}

/// A keyset cursor with its selector and key types erased: the statement a
/// plan composes, and its rows decoded into `R`.
// [spec:pgorm:def:exec.cursor.erased]
pub trait ErasedCursor<R, M: MetricsCollector = NoOpMetrics>: fmt::Debug + Send + Sync {
    /// [`Cursor::statement`] under `plan`.
    fn statement(&self, plan: &CursorPlan) -> Result<SelectStatement, Error>;

    /// [`Cursor::all`] under `plan`.
    fn all<'a>(
        &'a self,
        db: DatabaseRef<'a, M>,
        plan: CursorPlan,
    ) -> BoxFuture<'a, Result<Vec<R>, Error>>;
}

/// pgorm's cursor over a selector `S` and key `K`, made afresh for each plan
/// by `cursor`, since a cursor whose selector is not `Clone` is not, and the
/// decode of its items into rows `R`.
///
/// ```
/// use pgorm::{CursorPlan, CursorWindow, EntityTrait, ErasedCursor, KeyedRead, pgorm_query::IntoValueTuple, tests_cfg::cake};
///
/// let cursor: Box<dyn ErasedCursor<i32>> = Box::new(KeyedRead::new(
///     || cake::Entity::find().cursor_by(cake::Column::Id),
///     |cake: cake::Model| cake.id,
/// ));
/// let plan = CursorPlan { after: Some(10.into_value_tuple()), window: Some(CursorWindow::First(2)), ..CursorPlan::default() };
/// let sql = cursor.statement(&plan).unwrap().to_string();
/// assert!(sql.ends_with(r#"WHERE "cake"."id" > 10 ORDER BY "cake"."id" ASC LIMIT 2"#), "{sql}");
/// ```
// [spec:pgorm:def:exec.cursor.erased]
pub struct KeyedRead<S: SelectorTrait, K, R> {
    cursor: Box<dyn Fn() -> Cursor<S, K> + Send + Sync>,
    decode: Arc<dyn Fn(S::Item) -> R + Send + Sync>,
}

impl<S: SelectorTrait, K, R> KeyedRead<S, K, R> {
    /// The cursor `cursor` makes, each item decoded by `decode`.
    pub fn new(
        cursor: impl Fn() -> Cursor<S, K> + Send + Sync + 'static,
        decode: impl Fn(S::Item) -> R + Send + Sync + 'static,
    ) -> Self {
        Self {
            cursor: Box::new(cursor),
            decode: Arc::new(decode),
        }
    }

    fn planned(&self, plan: &CursorPlan) -> Cursor<S, K> {
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
            Some(CursorWindow::First(rows)) => {
                cursor.first(rows);
            }
            Some(CursorWindow::Last(rows)) => {
                cursor.last(rows);
            }
            None => {}
        }
        cursor
    }
}

impl<S: SelectorTrait, K, R> fmt::Debug for KeyedRead<S, K, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyedRead").finish_non_exhaustive()
    }
}

// [spec:pgorm:def:exec.cursor.erased]
impl<S, K, R, M> ErasedCursor<R, M> for KeyedRead<S, K, R>
where
    M: MetricsCollector,
    S: SelectorTrait + Send + Sync + 'static,
    S::Item: Send,
    K: Send + Sync + 'static,
    R: Send + 'static,
{
    fn statement(&self, plan: &CursorPlan) -> Result<SelectStatement, Error> {
        self.planned(plan).statement()
    }

    fn all<'a>(
        &'a self,
        db: DatabaseRef<'a, M>,
        plan: CursorPlan,
    ) -> BoxFuture<'a, Result<Vec<R>, Error>> {
        Box::pin(async move {
            let items = self.planned(&plan).all(&db).await?;
            Ok(items.into_iter().map(|item| (self.decode)(item)).collect())
        })
    }
}

/// A key an erased cursor cannot be made of, or a boundary it cannot take.
// [spec:pgorm:def:exec.cursor.erased]
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeyArity {
    /// A key of no column.
    #[error("a cursor orders by at least one column")]
    Empty,
    /// A graph cursor's key of more than [`MAX_KEY_COLUMNS`] columns.
    #[error("a graph's cursor orders by at most {MAX_KEY_COLUMNS} columns")]
    TooMany,
    /// A boundary of `given` values, where the cursor takes `expected`.
    #[error("a cursor boundary of {given} values does not match {expected} key column(s)")]
    Boundary {
        /// The values the boundary gives.
        given: usize,
        /// The counts it may give: the order columns', or that or the whole
        /// key's, as `"1"` or `"1 or 3"`.
        expected: String,
    },
}

impl From<KeyArity> for Error {
    fn from(arity: KeyArity) -> Self {
        Self::Type(arity.to_string())
    }
}

/// The columns a boundary of `given` values names, each value converted
/// through its column's type before the cursor takes it: the order columns'
/// when it gives one value per order column, or with `whole_key` — an
/// `after_with` or `before_with` — the whole key's when it gives one per key
/// column. Any other count is [`KeyArity::Boundary`], naming the counts the
/// cursor takes, as [`Cursor`] itself would refuse it.
///
/// ```
/// use pgorm::{KeyArity, boundary_columns};
///
/// let (order, key) = (["name"], ["name", "id"]);
/// assert_eq!(boundary_columns(&order, &key, 2, true), Ok(&key[..]));
/// assert_eq!(
///     boundary_columns(&order, &key, 2, false).unwrap_err().to_string(),
///     "a cursor boundary of 2 values does not match 1 key column(s)"
/// );
/// ```
// [spec:pgorm:def:exec.cursor.erased]
pub fn boundary_columns<'a, T>(
    order: &'a [T],
    key: &'a [T],
    given: usize,
    whole_key: bool,
) -> Result<&'a [T], KeyArity> {
    if given == order.len() {
        return Ok(order);
    }
    if whole_key && given == key.len() {
        return Ok(key);
    }
    let expected = if whole_key && key.len() > order.len() {
        format!("{} or {}", order.len(), key.len())
    } else {
        order.len().to_string()
    };
    Err(KeyArity::Boundary { given, expected })
}

/// [`Select::cursor_by`] over the columns `names`, chosen at run time, its
/// boundaries bound under each column's casts, erased; each model decoded by
/// `decode`. A plain read has no tiebreak, so its whole key is its order
/// columns. No column is [`KeyArity::Empty`].
// [spec:pgorm:def:exec.cursor.erased]
pub fn select_cursor<E, R, M>(
    select: Select<E>,
    names: Vec<Name>,
    decode: impl Fn(E::Model) -> R + Send + Sync + 'static,
) -> Result<Arc<dyn ErasedCursor<R, M>>, KeyArity>
where
    M: MetricsCollector,
    E: EntityTrait + Send + Sync + 'static,
    E::Model: FromQueryResult + Send + Sync,
    Select<E>: Clone + Send + Sync + 'static,
    R: Send + 'static,
{
    if names.is_empty() {
        return Err(KeyArity::Empty);
    }
    Ok(Arc::new(KeyedRead::new(
        move || select.clone().cursor_by(Key::from(names.clone())),
        decode,
    )))
}

/// A graph of one optional slot read grouped, [`SelectGraph::all_grouped`],
/// erased: each root with its slot's models, in the order roots first appear,
/// the root decoded by `root` and each slot model by `slot`.
// [spec:pgorm:def:exec.cursor.erased]
pub type GroupedRead<A, B, M = NoOpMetrics> = Box<
    dyn for<'a> Fn(DatabaseRef<'a, M>) -> BoxFuture<'a, Result<Vec<(A, Vec<B>)>, Error>>
        + Send
        + Sync,
>;

/// [`SelectGraph::all_grouped`] of `graph`, erased as [`GroupedRead`].
// [spec:pgorm:def:exec.cursor.erased]
pub fn grouped_read<E, F, A, B, M>(
    graph: SelectGraph<E, (Opt<F>,)>,
    root: impl Fn(E::Model) -> A + Send + Sync + 'static,
    slot: impl Fn(F::Model) -> B + Send + Sync + 'static,
) -> GroupedRead<A, B, M>
where
    M: MetricsCollector,
    E: EntityTrait + Send + Sync + 'static,
    F: EntityTrait + Send + Sync + 'static,
    SelectGraph<E, (Opt<F>,)>: Clone + Send + Sync + 'static,
    A: Send + 'static,
    B: Send + 'static,
{
    let (root, slot) = (Arc::new(root), Arc::new(slot));
    Box::new(move |db| {
        let (graph, root, slot) = (graph.clone(), root.clone(), slot.clone());
        Box::pin(async move {
            let groups = graph.all_grouped(&db).await?;
            Ok(groups
                .into_iter()
                .map(|(parent, children)| {
                    (
                        root(parent),
                        children.into_iter().map(|child| slot(child)).collect(),
                    )
                })
                .collect())
        })
    })
}

/// The erased cursor `$make` builds of `$graph` and a key of `$columns`, one
/// arm per arity, since each arity is a key type of its own.
macro_rules! by_arity {
    ($graph:ident, $columns:ident, $decode:ident, |$g:ident, $k:ident| $make:expr) => {
        match $columns {
            [a] => {
                let a = *a;
                Ok(Arc::new(KeyedRead::new(
                    move || {
                        let ($g, $k) = ($graph.clone(), a);
                        $make
                    },
                    $decode,
                )))
            }
            [a, b] => {
                let key = (*a, *b);
                Ok(Arc::new(KeyedRead::new(
                    move || {
                        let ($g, $k) = ($graph.clone(), key);
                        $make
                    },
                    $decode,
                )))
            }
            [a, b, c] => {
                let key = (*a, *b, *c);
                Ok(Arc::new(KeyedRead::new(
                    move || {
                        let ($g, $k) = ($graph.clone(), key);
                        $make
                    },
                    $decode,
                )))
            }
            [a, b, c, d] => {
                let key = (*a, *b, *c, *d);
                Ok(Arc::new(KeyedRead::new(
                    move || {
                        let ($g, $k) = ($graph.clone(), key);
                        $make
                    },
                    $decode,
                )))
            }
            [a, b, c, d, e] => {
                let key = (*a, *b, *c, *d, *e);
                Ok(Arc::new(KeyedRead::new(
                    move || {
                        let ($g, $k) = ($graph.clone(), key);
                        $make
                    },
                    $decode,
                )))
            }
            [a, b, c, d, e, f] => {
                let key = (*a, *b, *c, *d, *e, *f);
                Ok(Arc::new(KeyedRead::new(
                    move || {
                        let ($g, $k) = ($graph.clone(), key);
                        $make
                    },
                    $decode,
                )))
            }
            [] => Err(KeyArity::Empty),
            _ => Err(KeyArity::TooMany),
        }
    };
}

/// The most columns an erased graph cursor orders by: the arities
/// [`graph_cursor`] and [`graph_cursor_on`] build a key type for.
pub const MAX_KEY_COLUMNS: usize = 6;

/// [`SelectGraph::cursor_by`] over `columns` of the root, a key of one to
/// [`MAX_KEY_COLUMNS`] columns chosen at run time, erased; each row decoded by
/// `decode`. No column is [`KeyArity::Empty`], more than the most
/// [`KeyArity::TooMany`].
///
/// ```
/// use std::sync::Arc;
/// use pgorm::{CursorPlan, EntityTrait, ErasedCursor, RelationTrait, graph_cursor, tests_cfg::{cake, fruit}};
///
/// let graph = cake::Entity::graph().join_maybe::<fruit::Entity>(cake::Relation::Fruit.def());
/// let cursor: Arc<dyn ErasedCursor<i32>> =
///     graph_cursor(&graph, &[cake::Column::Name], |(cake, _): (cake::Model, Option<fruit::Model>)| cake.id).unwrap();
/// let sql = cursor.statement(&CursorPlan::default()).unwrap().to_string();
/// assert!(sql.ends_with(r#"ORDER BY "cake"."name" ASC, "cake"."id" ASC, "fruit"."id" ASC"#), "{sql}");
/// ```
// [spec:pgorm:def:exec.cursor.erased]
pub fn graph_cursor<E, S, R, M>(
    graph: &SelectGraph<E, S>,
    columns: &[E::Column],
    decode: impl Fn(GraphItem<E, S>) -> R + Send + Sync + 'static,
) -> Result<Arc<dyn ErasedCursor<R, M>>, KeyArity>
where
    M: MetricsCollector,
    E: EntityTrait + Send + Sync + 'static,
    S: Slots,
    SelectGraph<E, S>: Clone + Send + Sync + 'static,
    GraphRow<E, S>: SelectorTrait + Send + Sync + 'static,
    GraphItem<E, S>: Send,
    R: Send + 'static,
{
    let graph = graph.clone();
    by_arity!(graph, columns, decode, |g, k| g.cursor_by(k))
}

/// [`SelectGraph::cursor_by_on`] over `columns` of slot `I`, a key of one to
/// [`MAX_KEY_COLUMNS`] columns chosen at run time, erased, as
/// [`graph_cursor`] erases the root's.
// [spec:pgorm:def:exec.cursor.erased]
pub fn graph_cursor_on<const I: usize, E, S, R, M>(
    graph: &SelectGraph<E, S>,
    columns: &[<<<S as SlotAt<I>>::Slot as Slot>::Entity as EntityTrait>::Column],
    decode: impl Fn(GraphItem<E, S>) -> R + Send + Sync + 'static,
) -> Result<Arc<dyn ErasedCursor<R, M>>, KeyArity>
where
    M: MetricsCollector,
    E: EntityTrait + Send + Sync + 'static,
    S: SlotAt<I>,
    SelectGraph<E, S>: Clone + Send + Sync + 'static,
    GraphRow<E, S>: SelectorTrait + Send + Sync + 'static,
    GraphItem<E, S>: Send,
    R: Send + 'static,
{
    let graph = graph.clone();
    by_arity!(graph, columns, decode, |g, k| g.cursor_by_on::<I, _>(k))
}

/// The whole key a graph's cursor compares, as [`SelectGraph::cursor_by`]
/// (`ordered` 0) and [`SelectGraph::cursor_by_on`] (`ordered` the slot) install
/// it: the order `columns`, then the ordered source's primary key less any
/// column already ordered by, then every other source's primary key in source
/// order, the root first. `primary_keys` holds each decoded source's key
/// columns, the root's first; each entry names its source by index.
// [spec:pgorm:def:exec.cursor.erased]
pub fn graph_cursor_key<'a>(
    primary_keys: &[&'a [String]],
    ordered: usize,
    columns: &'a [String],
) -> Vec<(usize, &'a str)> {
    let own = primary_keys
        .get(ordered)
        .into_iter()
        .flat_map(|key| key.iter())
        .filter(|column| !columns.contains(column))
        .map(move |column| (ordered, column.as_str()));
    let others = primary_keys
        .iter()
        .enumerate()
        .filter(|(source, _)| *source != ordered)
        .flat_map(|(source, key)| key.iter().map(move |column| (source, column.as_str())));
    columns
        .iter()
        .map(|column| (ordered, column.as_str()))
        .chain(own)
        .chain(others)
        .collect()
}

// [spec:pgorm:def:exec.cursor.erased/test]    an erased graph cursor's SQL is the typed cursor's, and
// graph_cursor_key lists the columns that SQL orders by, in its order
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        RelationTrait,
        tests_cfg::{cake, fruit},
    };
    use pgorm_query::IntoValueTuple;

    fn graph() -> SelectGraph<cake::Entity, (crate::Opt<fruit::Entity>,)> {
        cake::Entity::graph().join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
    }

    #[test]
    fn erased_cursors_write_the_typed_cursors_sql() {
        let decode = |(cake, _): (cake::Model, Option<fruit::Model>)| cake.id;
        let plan = CursorPlan {
            after: Some(("Cheesecake", 1, 2).into_value_tuple()),
            window: Some(CursorWindow::Last(2)),
            ..CursorPlan::default()
        };
        let erased =
            graph_cursor::<_, _, _, NoOpMetrics>(&graph(), &[cake::Column::Name], decode).unwrap();
        let mut typed = graph().cursor_by(cake::Column::Name);
        typed.after_with(("Cheesecake", 1, 2)).last(2);
        assert_eq!(
            erased.statement(&plan).unwrap().to_string(),
            typed.statement().unwrap().to_string()
        );
        let on =
            graph_cursor_on::<1, _, _, _, NoOpMetrics>(&graph(), &[fruit::Column::Name], decode)
                .unwrap();
        let mut typed = graph().cursor_by_on::<1, _>(fruit::Column::Name);
        assert_eq!(
            on.statement(&CursorPlan::default()).unwrap().to_string(),
            typed.statement().unwrap().to_string()
        );
        assert!(graph_cursor::<_, _, _, NoOpMetrics>(&graph(), &[], decode).is_err());
        assert!(
            graph_cursor::<_, _, _, NoOpMetrics>(&graph(), &[cake::Column::Id; 7], decode).is_err()
        );
    }

    #[test]
    fn graph_key_is_the_order_the_cursor_writes() {
        let keys = [vec!["id".to_owned()], vec!["id".to_owned()]];
        let keys: Vec<&[String]> = keys.iter().map(Vec::as_slice).collect();
        let name = ["name".to_owned()];
        assert_eq!(
            graph_cursor_key(&keys, 0, &name),
            [(0, "name"), (0, "id"), (1, "id")]
        );
        assert_eq!(
            graph_cursor_key(&keys, 1, &name),
            [(1, "name"), (1, "id"), (0, "id")]
        );
        let id = ["id".to_owned()];
        assert_eq!(graph_cursor_key(&keys, 0, &id), [(0, "id"), (1, "id")]);

        let qualifiers = ["cake", "fruit"];
        for (ordered, columns) in [(0, &name), (1, &name), (0, &id)] {
            let expected: Vec<String> = graph_cursor_key(&keys, ordered, columns)
                .into_iter()
                .map(|(source, column)| format!(r#""{}"."{column}" ASC"#, qualifiers[source]))
                .collect();
            let mut cursor = if ordered == 0 {
                graph().cursor_by(if columns == &id {
                    cake::Column::Id
                } else {
                    cake::Column::Name
                })
            } else {
                graph().cursor_by_on::<1, _>(fruit::Column::Name)
            };
            let sql = cursor.statement().unwrap().to_string();
            let (_, order) = sql.split_once("ORDER BY ").unwrap();
            assert_eq!(order, expected.join(", "));
        }
    }
}
