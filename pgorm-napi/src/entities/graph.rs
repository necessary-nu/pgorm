//! Registered `SelectGraph<E, S>` shapes: the root entity and its tuple of
//! `Req<F>` and `Opt<F>` slots fixed in Rust, with a factory the application
//! supplies to build the graph under the aliases JavaScript chooses. Rust
//! writes the projection and decodes each row through `GraphRow`; JavaScript
//! varies filters, ordering, aliases and cursor bounds within that shape.

use std::{fmt, marker::PhantomData, sync::Arc};

use futures_util::future::BoxFuture;
use pgorm::pgorm_query::{Condition, NullOrdering, Order, SelectStatement, SimpleExpr, Values};
use pgorm::{
    EntityTrait, Error, GraphItem, GraphRow, IntoActiveModel, Opt, QueryFilter, QueryOrder,
    QueryTrait, Req, SelectGraph, SelectorTrait, Slot, Slots,
};

use super::{
    Registry,
    adapter::{Model, model_of},
    info::EntityInfo,
    keysets::{grouped_by, root_keyset, slot_keyset},
    reads::{Decode, Keyset, Pages},
    registry::RegistrationError,
};
use crate::connect::job::Database;

/// A grouped read's answer: each root with its slot's models.
pub(crate) type Groups = Vec<(Model, Vec<Model>)>;

/// `all_grouped` of a graph, pgorm's erased grouped read, run on whichever
/// database a job holds.
pub(crate) type GroupedFn = pgorm::GroupedRead<Model, Model, pgorm_bindings::Metrics>;

/// A registered graph's keyset cursor, built where the slot tuple's
/// positions are known.
#[doc(hidden)]
#[derive(Debug)]
pub struct SlotKeyset(pub(crate) Arc<Keyset>);

/// A registered graph's grouped read, which only a tuple of one `Opt` slot
/// has.
#[doc(hidden)]
pub struct GroupedRead(pub(crate) GroupedFn);

impl fmt::Debug for GroupedRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GroupedRead").finish_non_exhaustive()
    }
}

/// One decoded source of a registered graph: its entity, and whether its
/// row may be absent.
#[derive(Clone, Debug)]
pub struct Source {
    pub(crate) entity: Arc<EntityInfo>,
    pub(crate) optional: bool,
}

/// One decoded model of a graph row, behind its type-erased handle.
#[derive(Clone, Debug)]
pub struct GraphModel(pub(crate) Model);

/// The registered entities a graph's tuple decodes, root first. Produced by
/// [`GraphSlots`]; applications do not construct it.
#[derive(Clone, Debug)]
pub struct GraphBindings {
    pub(crate) sources: Vec<Source>,
}

#[doc(hidden)]
pub trait RegisteredSlot: Slot<Out: Send> {
    fn source(registry: &Registry) -> Result<Source, RegistrationError>;
    fn model(value: Self::Out, source: &Source) -> Option<GraphModel>;
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
    fn source(registry: &Registry) -> Result<Source, RegistrationError> {
        Ok(Source {
            entity: registry.registered::<F>()?,
            optional: false,
        })
    }

    fn model(value: Self::Out, source: &Source) -> Option<GraphModel> {
        Some(GraphModel(model_of::<F>(value, &source.entity)))
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
    fn source(registry: &Registry) -> Result<Source, RegistrationError> {
        Ok(Source {
            entity: registry.registered::<F>()?,
            optional: true,
        })
    }

    fn model(value: Self::Out, source: &Source) -> Option<GraphModel> {
        value.map(|value| GraphModel(model_of::<F>(value, &source.entity)))
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
    fn bindings(registry: &Registry) -> Result<GraphBindings, RegistrationError>;
    #[doc(hidden)]
    fn models(row: GraphItem<E, Self>, bindings: &GraphBindings) -> Vec<Option<GraphModel>>;
    /// `cursor_by` over root columns for source 0, `cursor_by_on::<I, _>`
    /// over slot `I`'s otherwise; the refusal's message when there is none.
    #[doc(hidden)]
    fn keyset(
        graph: &SelectGraph<E, Self>,
        source: usize,
        columns: &[String],
        bindings: &GraphBindings,
    ) -> Result<SlotKeyset, String>;
    #[doc(hidden)]
    fn grouped(graph: SelectGraph<E, Self>, bindings: &GraphBindings) -> Option<GroupedRead>;
}

fn no_source(source: usize) -> String {
    format!("the graph has no slot {source}")
}

impl<E> GraphSlots<E> for ()
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn bindings(registry: &Registry) -> Result<GraphBindings, RegistrationError> {
        Ok(GraphBindings {
            sources: vec![Source {
                entity: registry.registered::<E>()?,
                optional: false,
            }],
        })
    }

    fn models(value: E::Model, bindings: &GraphBindings) -> Vec<Option<GraphModel>> {
        vec![
            bindings
                .sources
                .first()
                .map(|source| GraphModel(model_of::<E>(value, &source.entity))),
        ]
    }

    fn keyset(
        graph: &SelectGraph<E, Self>,
        source: usize,
        columns: &[String],
        bindings: &GraphBindings,
    ) -> Result<SlotKeyset, String> {
        match source {
            0 => root_keyset(graph, columns, bindings).map(SlotKeyset),
            other => Err(no_source(other)),
        }
    }

    fn grouped(_: SelectGraph<E, Self>, _: &GraphBindings) -> Option<GroupedRead> {
        None
    }
}

macro_rules! slots {
    ($grouped:ident; $($slot:ident @ $at:tt),+) => {
        impl<E, $($slot),+> GraphSlots<E> for ($($slot,)+)
        where
            E: EntityTrait + Send + Sync + 'static,
            E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
            E::ActiveModel: Send + Sync + 'static,
            $($slot: RegisteredSlot + fmt::Debug + Send + Sync + 'static,)+
        {
            fn bindings(registry: &Registry) -> Result<GraphBindings, RegistrationError> {
                Ok(GraphBindings {
                    sources: vec![
                        Source { entity: registry.registered::<E>()?, optional: false },
                        $($slot::source(registry)?,)+
                    ],
                })
            }

            fn models(row: GraphItem<E, Self>, bindings: &GraphBindings) -> Vec<Option<GraphModel>> {
                let sources = &bindings.sources;
                vec![
                    sources.first().map(|source| GraphModel(model_of::<E>(row.0, &source.entity))),
                    $(sources.get($at).and_then(|source| $slot::model(row.$at, source)),)+
                ]
            }

            fn keyset(
                graph: &SelectGraph<E, Self>,
                source: usize,
                columns: &[String],
                bindings: &GraphBindings,
            ) -> Result<SlotKeyset, String> {
                match source {
                    0 => root_keyset(graph, columns, bindings).map(SlotKeyset),
                    $($at => slot_keyset::<$at, E, Self>(graph, columns, bindings).map(SlotKeyset),)+
                    other => Err(no_source(other)),
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

/// A registered graph: its name, the Rust shape and its sources.
#[derive(Debug)]
pub(crate) struct GraphInfo {
    pub(crate) name: String,
    pub(crate) shape: &'static str,
    pub(crate) bindings: GraphBindings,
}

impl GraphInfo {
    /// The graph as `describe()` reports it: its name, the Rust shape, and
    /// each source's entity and slot kind.
    // [spec:pgorm:req:bindings.entity-graphs+4]
    pub(crate) fn describe(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "rustShape": self.shape,
            "sources": self.bindings.sources.iter().enumerate().map(|(index, source)| {
                serde_json::json!({
                    "index": index,
                    "entity": source.entity.name,
                    "slot": if index == 0 { "root" } else if source.optional { "Opt" } else { "Req" },
                })
            }).collect::<Vec<_>>(),
        })
    }
}

pub(crate) type Query = Arc<dyn QueryBackend>;
pub(crate) type Row = Vec<Option<GraphModel>>;

pub(crate) trait Factory: fmt::Debug + Send + Sync {
    fn info(&self) -> &Arc<GraphInfo>;
    fn find(&self, aliases: Vec<String>) -> Query;
}

/// One step a graph query takes, each the `SelectGraph` method of its
/// meaning.
#[derive(Clone, Debug)]
pub(crate) enum Change {
    Filter(Condition),
    Order(SimpleExpr, Order, Option<NullOrdering>),
}

pub(crate) trait QueryBackend: fmt::Debug + Send + Sync {
    fn info(&self) -> &Arc<GraphInfo>;
    fn aliases(&self) -> &[String];
    fn change(&self, change: Change) -> Query;
    fn compile(&self, optional: bool) -> (String, Values);
    fn run<'a>(
        &'a self,
        db: Database<'a>,
        optional: bool,
    ) -> BoxFuture<'a, Result<Vec<Row>, Error>>;
    /// `cursor_by` over root columns when `source` is 0, otherwise
    /// `cursor_by_on` over the columns of slot `source`.
    fn keyset(&self, source: usize, columns: &[String]) -> Result<Arc<Keyset>, String>;
    fn pages(&self) -> Arc<Pages>;
    fn statement(&self) -> SelectStatement;
    /// The decode `SelectGraph::stream` runs on each row.
    fn decode(&self) -> Decode;
    /// `all_grouped`, which only a graph of one `Opt` slot has.
    fn grouped(&self) -> Option<GroupedFn>;
}

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

impl<E, S, Build> Factory for GraphFactory<E, S, Build>
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

    fn find(&self, aliases: Vec<String>) -> Query {
        Arc::new(GraphQuery::<E, S> {
            query: (self.build)(&aliases),
            info: self.info.clone(),
            aliases,
        })
    }
}

pub(crate) struct GraphQuery<E: EntityTrait, S> {
    pub(crate) query: SelectGraph<E, S>,
    pub(crate) info: Arc<GraphInfo>,
    pub(crate) aliases: Vec<String>,
}

impl<E: EntityTrait, S> fmt::Debug for GraphQuery<E, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphQuery")
            .field("info", &self.info)
            .field("aliases", &self.aliases)
            .finish_non_exhaustive()
    }
}

// [spec:pgorm:req:bindings.entity-graphs+4]
impl<E, S> QueryBackend for GraphQuery<E, S>
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

    fn change(&self, change: Change) -> Query {
        let query = self.query.clone();
        let query = match change {
            Change::Filter(condition) => query.filter(condition),
            Change::Order(expr, order, None) => query.order_by(expr, order),
            Change::Order(expr, order, Some(nulls)) => {
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

    fn keyset(&self, source: usize, columns: &[String]) -> Result<Arc<Keyset>, String> {
        S::keyset(&self.query, source, columns, &self.info.bindings).map(|keyset| keyset.0)
    }

    fn pages(&self) -> Arc<Pages> {
        let info = self.info.clone();
        Arc::new(Pages {
            pages: Box::new(pgorm::PagedRead::new(self.query.clone(), move |row| {
                S::models(row, &info.bindings)
                    .into_iter()
                    .map(|model| model.map(|model| model.0))
                    .collect()
            })),
            sources: self
                .info
                .bindings
                .sources
                .iter()
                .map(|source| source.entity.clone())
                .collect(),
        })
    }

    fn statement(&self) -> SelectStatement {
        QueryTrait::as_query(&self.query).clone()
    }

    fn decode(&self) -> Decode {
        let info = self.info.clone();
        Arc::new(move |row| {
            let item = GraphRow::<E, S>::from_raw_query_result(row)?;
            Ok(S::models(item, &info.bindings)
                .into_iter()
                .map(|model| model.map(|model| model.0))
                .collect())
        })
    }

    fn grouped(&self) -> Option<GroupedFn> {
        S::grouped(self.query.clone(), &self.info.bindings).map(|read| read.0)
    }
}
