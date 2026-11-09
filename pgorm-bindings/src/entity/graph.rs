//! Registered `SelectGraph<E, S>` shapes: the root entity and its tuple of
//! `Req<F>` and `Opt<F>` slots fixed in Rust, with a factory the application
//! supplies to build the graph under the aliases a program chooses. Rust
//! writes the projection and decodes each row through `GraphRow`; the
//! program varies filters, ordering, aliases and cursor bounds within that
//! shape.

use std::{fmt, marker::PhantomData, sync::Arc};

use futures_util::future::BoxFuture;
use pgorm::pgorm_query::{
    Condition, Expr, Name, NullOrdering, Order, SelectStatement, SimpleExpr, Values,
};
use pgorm::{
    EntityTrait, ErasedCursor, Error, GraphItem, GraphRow, IntoActiveModel, Opt, QueryFilter,
    QueryOrder, QueryTrait, Req, SelectGraph, SelectorTrait, Slot, SlotAt, Slots, graph_cursor,
    graph_cursor_key, graph_cursor_on, grouped_read,
};
use serde_json::Value as Json;

use super::{
    EntityInfo, Model, Registry,
    adapter::column,
    model_of, object,
    reads::{Decode, KeyColumn, Keyset, Pages, Row},
};
use crate::{Database, Dialect, Failure, Metrics};

/// `all_grouped` of a graph, pgorm's erased grouped read: each root with its
/// slot's models.
pub type Grouped = pgorm::GroupedRead<Model, Model, Metrics>;

/// A registered graph's grouped read, which only a tuple of one `Opt` slot
/// has.
#[doc(hidden)]
pub struct GroupedRead(Grouped);

impl fmt::Debug for GroupedRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GroupedRead").finish_non_exhaustive()
    }
}

/// One decoded source of a registered graph: its entity, and whether its
/// row may be absent.
#[derive(Clone, Debug)]
pub struct Source {
    /// The registered entity the source's row decodes as.
    pub entity: Arc<EntityInfo>,
    /// Whether the source is an `Opt` slot, whose row may be absent.
    pub optional: bool,
}

/// The registered entities a graph's tuple decodes, root first. Produced by
/// [`GraphSlots`]; applications do not construct it.
#[derive(Clone, Debug)]
pub struct GraphBindings {
    /// The root, then each slot in tuple order.
    pub sources: Vec<Source>,
}

impl GraphBindings {
    /// The entities each row's sources decode as.
    pub fn entities(&self) -> Vec<Arc<EntityInfo>> {
        self.sources
            .iter()
            .map(|source| source.entity.clone())
            .collect()
    }
}

/// A slot of a registered graph's tuple: `Req<F>` or `Opt<F>` of a
/// registered entity `F`.
#[doc(hidden)]
pub trait RegisteredSlot: Slot<Out: Send> {
    fn source(registry: &Registry) -> Result<Source, Failure>;
    fn model(value: Self::Out, source: &Source) -> Option<Model>;
    /// `all_grouped` of a graph whose one slot this is; `None` for a `Req`
    /// slot, which Rust's `all_grouped` is not on.
    fn grouped<E>(graph: SelectGraph<E, (Self,)>, bindings: &GraphBindings) -> Option<GroupedRead>
    where
        E: EntityTrait + Send + Sync + 'static,
        E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
        E::ActiveModel: Send + Sync + 'static,
        Self: Sized + Send + Sync + 'static;
}

impl<F> RegisteredSlot for Req<F>
where
    F: EntityTrait + Send + Sync + 'static,
    F::Model: IntoActiveModel<F::ActiveModel> + Sync + 'static,
    F::ActiveModel: Send + Sync + 'static,
{
    fn source(registry: &Registry) -> Result<Source, Failure> {
        Ok(Source {
            entity: registry.registered::<F>()?,
            optional: false,
        })
    }

    fn model(value: Self::Out, source: &Source) -> Option<Model> {
        Some(model_of::<F>(value, &source.entity))
    }

    fn grouped<E>(_: SelectGraph<E, (Self,)>, _: &GraphBindings) -> Option<GroupedRead>
    where
        E: EntityTrait + Send + Sync + 'static,
        E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
        E::ActiveModel: Send + Sync + 'static,
    {
        None
    }
}

impl<F> RegisteredSlot for Opt<F>
where
    F: EntityTrait + Send + Sync + 'static,
    F::Model: IntoActiveModel<F::ActiveModel> + Sync + 'static,
    F::ActiveModel: Send + Sync + 'static,
{
    fn source(registry: &Registry) -> Result<Source, Failure> {
        Ok(Source {
            entity: registry.registered::<F>()?,
            optional: true,
        })
    }

    fn model(value: Self::Out, source: &Source) -> Option<Model> {
        value.map(|value| model_of::<F>(value, &source.entity))
    }

    fn grouped<E>(graph: SelectGraph<E, (Self,)>, bindings: &GraphBindings) -> Option<GroupedRead>
    where
        E: EntityTrait + Send + Sync + 'static,
        E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
        E::ActiveModel: Send + Sync + 'static,
    {
        Some(GroupedRead(grouped_by::<E, F>(graph, bindings)))
    }
}

/// The slot tuples a graph can register: Rust's own arities, from no slot to
/// six. pgorm's `Slots` is sealed, so no other shape implements this.
// [spec:pgorm:req:bindings.entity-graphs+4]
pub trait GraphSlots<E: EntityTrait>: Slots + fmt::Debug + Send + Sync + Sized + 'static
where
    GraphRow<E, Self>: SelectorTrait,
{
    #[doc(hidden)]
    fn bindings(registry: &Registry) -> Result<GraphBindings, Failure>;
    #[doc(hidden)]
    fn models(row: GraphItem<E, Self>, bindings: &GraphBindings) -> Row;
    /// `cursor_by` over root columns for source 0, `cursor_by_on::<I, _>`
    /// over slot `I`'s otherwise.
    #[doc(hidden)]
    fn keyset(
        graph: &SelectGraph<E, Self>,
        source: usize,
        columns: &[String],
        bindings: &GraphBindings,
    ) -> Result<Arc<Keyset>, Failure>;
    #[doc(hidden)]
    fn grouped(graph: SelectGraph<E, Self>, bindings: &GraphBindings) -> Option<GroupedRead>;
}

/// The root's model of a graph row.
fn root<E>(value: E::Model, bindings: &GraphBindings) -> Option<Model>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    bindings
        .sources
        .first()
        .map(|source| model_of::<E>(value, &source.entity))
}

// [spec:pgorm:req:bindings.entity-graphs+4]
impl<E> GraphSlots<E> for ()
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn bindings(registry: &Registry) -> Result<GraphBindings, Failure> {
        Ok(GraphBindings {
            sources: vec![Source {
                entity: registry.registered::<E>()?,
                optional: false,
            }],
        })
    }

    fn models(value: E::Model, bindings: &GraphBindings) -> Row {
        vec![root::<E>(value, bindings)]
    }

    fn keyset(
        graph: &SelectGraph<E, Self>,
        source: usize,
        columns: &[String],
        bindings: &GraphBindings,
    ) -> Result<Arc<Keyset>, Failure> {
        match source {
            0 => root_keyset(graph, columns, bindings),
            other => Err(Failure::Construction(format!(
                "the graph has no slot {other}"
            ))),
        }
    }

    fn grouped(_: SelectGraph<E, Self>, _: &GraphBindings) -> Option<GroupedRead> {
        None
    }
}

macro_rules! slots {
    ($grouped:ident; $($slot:ident @ $at:tt),+) => {
        // [spec:pgorm:req:bindings.entity-graphs+4]
        impl<E, $($slot),+> GraphSlots<E> for ($($slot,)+)
        where
            E: EntityTrait + Send + Sync + 'static,
            E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
            E::ActiveModel: Send + Sync + 'static,
            $($slot: RegisteredSlot + fmt::Debug + Send + Sync + 'static,)+
        {
            fn bindings(registry: &Registry) -> Result<GraphBindings, Failure> {
                Ok(GraphBindings {
                    sources: vec![
                        Source { entity: registry.registered::<E>()?, optional: false },
                        $($slot::source(registry)?,)+
                    ],
                })
            }

            fn models(row: GraphItem<E, Self>, bindings: &GraphBindings) -> Row {
                let sources = &bindings.sources;
                vec![
                    root::<E>(row.0, bindings),
                    $(sources.get($at).and_then(|source| $slot::model(row.$at, source)),)+
                ]
            }

            fn keyset(
                graph: &SelectGraph<E, Self>,
                source: usize,
                columns: &[String],
                bindings: &GraphBindings,
            ) -> Result<Arc<Keyset>, Failure> {
                match source {
                    0 => root_keyset(graph, columns, bindings),
                    $($at => slot_keyset::<$at, E, Self>(graph, columns, bindings),)+
                    other => Err(Failure::Construction(format!("the graph has no slot {other}"))),
                }
            }

            fn grouped(graph: SelectGraph<E, Self>, bindings: &GraphBindings) -> Option<GroupedRead> {
                slots!(@grouped $grouped graph bindings $($slot)+)
            }
        }
    };
    (@grouped one $graph:ident $bindings:ident $slot:ident) => {
        <$slot as RegisteredSlot>::grouped($graph, $bindings)
    };
    (@grouped many $graph:ident $bindings:ident $($slot:ident)+) => {{
        let _ = ($graph, $bindings);
        None
    }};
}

slots!(one; S1 @ 1);
slots!(many; S1 @ 1, S2 @ 2);
slots!(many; S1 @ 1, S2 @ 2, S3 @ 3);
slots!(many; S1 @ 1, S2 @ 2, S3 @ 3, S4 @ 4);
slots!(many; S1 @ 1, S2 @ 2, S3 @ 3, S4 @ 4, S5 @ 5);
slots!(many; S1 @ 1, S2 @ 2, S3 @ 3, S4 @ 4, S5 @ 5, S6 @ 6);

/// The order columns of source `ordered`, and the whole key a cursor over
/// them compares, as pgorm's `graph_cursor_key` lists it, each column with
/// the entity it converts a boundary value through.
fn key_of(
    bindings: &GraphBindings,
    ordered: usize,
    columns: &[String],
) -> (Vec<KeyColumn>, Vec<KeyColumn>) {
    let primary_keys: Vec<&[String]> = bindings
        .sources
        .iter()
        .map(|source| source.entity.primary_key.as_slice())
        .collect();
    let named = |(source, column): (usize, &str)| KeyColumn {
        entity: bindings.sources[source].entity.clone(),
        column: column.to_owned(),
    };
    let key: Vec<_> = graph_cursor_key(&primary_keys, ordered, columns)
        .into_iter()
        .map(named)
        .collect();
    (key[..columns.len()].to_vec(), key)
}

fn decoder<E, S>(
    bindings: &GraphBindings,
) -> impl Fn(GraphItem<E, S>) -> Row + Send + Sync + 'static
where
    E: EntityTrait + Send + Sync + 'static,
    S: GraphSlots<E>,
    GraphRow<E, S>: SelectorTrait,
{
    let bindings = bindings.clone();
    move |item| S::models(item, &bindings)
}

fn keyset(
    bindings: &GraphBindings,
    ordered: usize,
    columns: &[String],
    cursor: Arc<dyn ErasedCursor<Row, Metrics>>,
) -> Arc<Keyset> {
    let (order, key) = key_of(bindings, ordered, columns);
    Arc::new(Keyset {
        cursor,
        order,
        key,
        sources: bindings.entities(),
    })
}

/// `SelectGraph::cursor_by` over root columns.
// [spec:pgorm:req:bindings.entity-graphs+4]
fn root_keyset<E, S>(
    graph: &SelectGraph<E, S>,
    columns: &[String],
    bindings: &GraphBindings,
) -> Result<Arc<Keyset>, Failure>
where
    E: EntityTrait + Send + Sync + 'static,
    S: GraphSlots<E>,
    GraphRow<E, S>: SelectorTrait + Send + Sync + 'static,
    GraphItem<E, S>: Send,
{
    let resolved = columns
        .iter()
        .map(|name| column::<E>(&bindings.sources[0].entity, name))
        .collect::<Result<Vec<_>, _>>()?;
    let cursor = graph_cursor(graph, &resolved, decoder::<E, S>(bindings))
        .map_err(|arity| Failure::Construction(arity.to_string()))?;
    Ok(keyset(bindings, 0, columns, cursor))
}

/// `SelectGraph::cursor_by_on::<I, _>` over slot `I`'s columns.
// [spec:pgorm:req:bindings.entity-graphs+4]
fn slot_keyset<const I: usize, E, S>(
    graph: &SelectGraph<E, S>,
    columns: &[String],
    bindings: &GraphBindings,
) -> Result<Arc<Keyset>, Failure>
where
    E: EntityTrait + Send + Sync + 'static,
    S: GraphSlots<E> + SlotAt<I>,
    GraphRow<E, S>: SelectorTrait + Send + Sync + 'static,
    GraphItem<E, S>: Send,
{
    let resolved = columns
        .iter()
        .map(|name| {
            column::<<<S as SlotAt<I>>::Slot as Slot>::Entity>(&bindings.sources[I].entity, name)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let cursor = graph_cursor_on::<I, _, _, _, _>(graph, &resolved, decoder::<E, S>(bindings))
        .map_err(|arity| Failure::Construction(arity.to_string()))?;
    Ok(keyset(bindings, I, columns, cursor))
}

/// `all_grouped` of a graph of one `Opt` slot, pgorm's `grouped_read`: each
/// root with its slot's models, in the order roots first appear.
// [spec:pgorm:req:bindings.entity-graphs+4]
fn grouped_by<E, F>(graph: SelectGraph<E, (Opt<F>,)>, bindings: &GraphBindings) -> Grouped
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
    F: EntityTrait + Send + Sync + 'static,
    F::Model: IntoActiveModel<F::ActiveModel> + Sync + 'static,
    F::ActiveModel: Send + Sync + 'static,
{
    let (root, slot) = (
        bindings.sources[0].entity.clone(),
        bindings.sources[1].entity.clone(),
    );
    grouped_read(
        graph,
        move |parent| model_of::<E>(parent, &root),
        move |child| model_of::<F>(child, &slot),
    )
}

/// A registered graph: its name, the Rust shape, its sources, and the
/// binding whose spelling its description and refusals take.
#[derive(Debug)]
pub struct GraphInfo {
    /// The name the graph is registered under.
    pub name: String,
    /// The Rust type of the graph, `SelectGraph<E, S>`.
    pub shape: &'static str,
    /// The graph's slots and the decoded sources each reads.
    pub bindings: GraphBindings,
    pub(crate) dialect: Dialect,
}

impl GraphInfo {
    /// The graph as `describe()` reports it: its name, the Rust shape, and
    /// each source's entity and slot kind.
    // [spec:pgorm:req:bindings.entity-graphs+4]
    pub fn describe(&self) -> Json {
        let sources = self
            .bindings
            .sources
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let slot = match (index, source.optional) {
                    (0, _) => "root",
                    (_, true) => "Opt",
                    (_, false) => "Req",
                };
                object(
                    self.dialect,
                    [
                        ("index", index.into()),
                        ("entity", source.entity.name.clone().into()),
                        ("slot", slot.into()),
                    ],
                )
            })
            .collect::<Vec<_>>();
        object(
            self.dialect,
            [
                ("name", self.name.clone().into()),
                ("rust_shape", self.shape.into()),
                ("sources", sources.into()),
            ],
        )
    }

    /// The aliases the graph's slots are joined under: the ones `given`, one
    /// per slot, each an identifier, none repeated nor the root's table; or
    /// else `g1`, `g2`, .., a name the root's table takes skipped.
    // [spec:pgorm:req:bindings.entity-graphs+4]
    pub fn aliases(&self, given: Option<Vec<String>>) -> Result<Vec<String>, Failure> {
        let sources = &self.bindings.sources;
        let root = sources
            .first()
            .map(|source| source.entity.table.as_str())
            .unwrap_or_default();
        let slots = sources.len().saturating_sub(1);
        let Some(given) = given else {
            return Ok((1..)
                .map(|at| format!("g{at}"))
                .filter(|name| name != root)
                .take(slots)
                .collect());
        };
        if given.len() != slots {
            return Err(Failure::Construction(format!(
                "the graph joins {slots} slots, and {} aliases were given",
                given.len()
            )));
        }
        let mut names: Vec<String> = Vec::with_capacity(slots);
        for name in given {
            if name.is_empty() || name.len() > 63 || name.contains('\0') {
                return Err(Failure::Construction(
                    "an alias is 1–63 UTF-8 bytes without NUL".to_owned(),
                ));
            }
            if name == root || names.contains(&name) {
                return Err(Failure::Construction(format!(
                    "the alias {name:?} names a source twice"
                )));
            }
            names.push(name);
        }
        Ok(names)
    }

    /// The decoded source `source`, refused when the graph has none.
    pub fn source(&self, source: usize) -> Result<&Source, Failure> {
        self.bindings
            .sources
            .get(source)
            .ok_or_else(|| Failure::Construction(format!("the graph has no source {source}")))
    }
}

/// A registered graph's query behind its type-erased handle.
pub type GraphQuery = Arc<dyn GraphQueryBackend>;

/// A registered graph shape: what it is, and the query its factory builds.
pub trait GraphBackend: fmt::Debug + Send + Sync {
    /// What the graph is.
    fn info(&self) -> &Arc<GraphInfo>;
    /// The graph built by the application's factory under `aliases`, one
    /// per slot.
    fn find(&self, aliases: Vec<String>) -> GraphQuery;
}

/// One step a graph query takes, each the `SelectGraph` method of its
/// meaning.
#[derive(Clone, Debug)]
pub enum GraphChange {
    /// `QueryFilter::filter`: the rows the condition keeps.
    Filter(Condition),
    /// `QueryOrder::order_by`, or `order_by_with_nulls` with a placement.
    Order(SimpleExpr, Order, Option<NullOrdering>),
}

/// A registered graph's query.
pub trait GraphQueryBackend: fmt::Debug + Send + Sync {
    /// What the graph is.
    fn info(&self) -> &Arc<GraphInfo>;
    /// The aliases the slots are joined under, in slot order.
    fn aliases(&self) -> &[String];
    /// The query with `change` applied, this one left as it is.
    fn change(&self, change: GraphChange) -> GraphQuery;
    /// The statement, `optional` narrowed to one row.
    fn compile(&self, optional: bool) -> (String, Values);
    /// `SelectGraph::all`, or `one_opt` when `optional`.
    fn run<'a>(
        &'a self,
        db: Database<'a>,
        optional: bool,
    ) -> BoxFuture<'a, Result<Vec<Row>, Error>>;
    /// `cursor_by` over root columns when `source` is 0, otherwise
    /// `cursor_by_on` over the columns of slot `source`; a source the graph
    /// lacks and a column its entity lacks are refused.
    fn keyset(&self, source: usize, columns: &[String]) -> Result<Arc<Keyset>, Failure>;
    /// The graph's pages, through pgorm's erased paginator.
    fn pages(&self) -> Arc<Pages>;
    /// The statement `SelectGraph::stream` sends.
    fn statement(&self) -> SelectStatement;
    /// The decode `SelectGraph::stream` runs on each row.
    fn decode(&self) -> Decode;
    /// `all_grouped`, which only a graph of one `Opt` slot has.
    fn grouped(&self) -> Result<Grouped, Failure>;

    /// A column of decoded source `source`, qualified as the query names
    /// it: the root by its table, a slot by its alias.
    // [spec:pgorm:req:bindings.entity-graphs+4]
    fn column(&self, source: usize, column: &str) -> Result<SimpleExpr, Failure> {
        let found = self.info().source(source)?;
        found.entity.column(column)?;
        let qualifier = match source {
            0 => found.entity.table.clone(),
            _ => self.aliases()[source - 1].clone(),
        };
        Ok(Expr::col((Name::runtime(qualifier), Name::runtime(column))).into())
    }
}

/// The registered shape of `SelectGraph<E, S>` and its factory.
pub(crate) struct GraphFactory<E: EntityTrait, S, Build> {
    pub(crate) info: Arc<GraphInfo>,
    pub(crate) build: Build,
    pub(crate) marker: PhantomData<(E, S)>,
}

impl<E: EntityTrait, S, Build> fmt::Debug for GraphFactory<E, S, Build> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphFactory")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl<E, S, Build> GraphBackend for GraphFactory<E, S, Build>
where
    E: EntityTrait + Send + Sync + 'static,
    S: GraphSlots<E>,
    GraphRow<E, S>: SelectorTrait + Send + Sync + 'static,
    GraphItem<E, S>: Send + 'static,
    Build: Fn(&[String]) -> SelectGraph<E, S> + Send + Sync + 'static,
{
    fn info(&self) -> &Arc<GraphInfo> {
        &self.info
    }

    fn find(&self, aliases: Vec<String>) -> GraphQuery {
        Arc::new(GraphRead::<E, S> {
            query: (self.build)(&aliases),
            info: self.info.clone(),
            aliases,
        })
    }
}

/// A registered graph's query, the real `SelectGraph<E, S>`.
struct GraphRead<E: EntityTrait, S> {
    query: SelectGraph<E, S>,
    info: Arc<GraphInfo>,
    aliases: Vec<String>,
}

impl<E: EntityTrait, S> fmt::Debug for GraphRead<E, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphRead")
            .field("info", &self.info)
            .field("aliases", &self.aliases)
            .finish_non_exhaustive()
    }
}

// [spec:pgorm:req:bindings.entity-graphs+4]
impl<E, S> GraphQueryBackend for GraphRead<E, S>
where
    E: EntityTrait + Send + Sync + 'static,
    S: GraphSlots<E>,
    GraphRow<E, S>: SelectorTrait + Send + Sync + 'static,
    GraphItem<E, S>: Send + 'static,
{
    fn info(&self) -> &Arc<GraphInfo> {
        &self.info
    }

    fn aliases(&self) -> &[String] {
        &self.aliases
    }

    fn change(&self, change: GraphChange) -> GraphQuery {
        let query = self.query.clone();
        let query = match change {
            GraphChange::Filter(condition) => query.filter(condition),
            GraphChange::Order(expr, order, None) => query.order_by(expr, order),
            GraphChange::Order(expr, order, Some(nulls)) => {
                query.order_by_with_nulls(expr, order, nulls)
            }
        };
        Arc::new(Self {
            query,
            info: self.info.clone(),
            aliases: self.aliases.clone(),
        })
    }

    fn compile(&self, optional: bool) -> (String, Values) {
        let mut query = self.query.clone();
        if optional {
            QueryTrait::query(&mut query).narrow_limit_to_one();
        }
        query.build()
    }

    fn run<'a>(
        &'a self,
        db: Database<'a>,
        optional: bool,
    ) -> BoxFuture<'a, Result<Vec<Row>, Error>> {
        Box::pin(async move {
            let rows = if optional {
                self.query.clone().one_opt(&db).await?.into_iter().collect()
            } else {
                self.query.clone().all(&db).await?
            };
            Ok(rows
                .into_iter()
                .map(|row| S::models(row, &self.info.bindings))
                .collect())
        })
    }

    fn keyset(&self, source: usize, columns: &[String]) -> Result<Arc<Keyset>, Failure> {
        let found = self.info.source(source)?;
        for column in columns {
            found.entity.column(column)?;
        }
        S::keyset(&self.query, source, columns, &self.info.bindings)
    }

    fn pages(&self) -> Arc<Pages> {
        let info = self.info.clone();
        Arc::new(Pages {
            pages: Box::new(pgorm::PagedRead::new(self.query.clone(), move |row| {
                S::models(row, &info.bindings)
            })),
            sources: self.info.bindings.entities(),
        })
    }

    fn statement(&self) -> SelectStatement {
        QueryTrait::as_query(&self.query).clone()
    }

    fn decode(&self) -> Decode {
        let info = self.info.clone();
        Arc::new(move |row| {
            let item = GraphRow::<E, S>::from_raw_query_result(row)?;
            Ok(S::models(item, &info.bindings))
        })
    }

    fn grouped(&self) -> Result<Grouped, Failure> {
        S::grouped(self.query.clone(), &self.info.bindings)
            .map(|read| read.0)
            .ok_or_else(|| {
                Failure::Construction(format!(
                    "{} reads a graph of one Opt slot, as Rust's does",
                    self.info.dialect.option("all_grouped")
                ))
            })
    }
}
