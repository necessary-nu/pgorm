use std::sync::Arc;

use pgorm::{
    EntityTrait, GraphItem, GraphRow, IntoActiveModel, Opt, Req, SelectGraph, SelectorTrait, Slot,
    Slots,
};
use pyo3::prelude::*;

use super::keysets::{grouped_by, root_keyset, slot_keyset};
use crate::entities::{
    PyEntityModel, Registry, generic::ModelAdapter, metadata::EntityInfo, reads::Keyset,
};
use crate::{errors::ConstructionError, transactions::work::Read};

#[derive(Clone, Debug)]
pub(crate) struct Source {
    pub(crate) entity: Arc<EntityInfo>,
    pub(crate) optional: bool,
}

/// The registered entity identities of a concrete Rust graph tuple.
/// Produced by GraphSlots; applications do not construct this value.
#[derive(Clone, Debug)]
pub struct GraphBindings {
    pub(crate) sources: Vec<Source>,
}

/// A registered graph's keyset cursor, built where the slot tuple's
/// positions are known.
#[doc(hidden)]
#[derive(Debug)]
pub struct SlotKeyset(pub(crate) Arc<dyn Keyset>);

/// A registered graph's grouped read, which only a tuple of one `Opt` slot
/// has.
#[doc(hidden)]
pub struct GroupedRead(pub(crate) Read);

impl std::fmt::Debug for GroupedRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupedRead").finish_non_exhaustive()
    }
}

pub(crate) trait RegisteredSlot: Slot<Out: Send> {
    fn source(registry: &Registry) -> PyResult<Source>;
    fn model(value: Self::Out, source: &Source) -> Option<PyEntityModel>;
    /// `all_grouped` of a graph whose one slot this is.
    fn grouped<E>(graph: SelectGraph<E, (Self,)>, bindings: &GraphBindings) -> PyResult<Read>
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
    fn source(registry: &Registry) -> PyResult<Source> {
        Ok(Source {
            entity: registry.registered::<F>()?,
            optional: false,
        })
    }
    fn model(value: Self::Out, source: &Source) -> Option<PyEntityModel> {
        Some(PyEntityModel {
            inner: Arc::new(ModelAdapter::<F> {
                value,
                info: source.entity.clone(),
            }),
        })
    }
    fn grouped<E>(_: SelectGraph<E, (Self,)>, _: &GraphBindings) -> PyResult<Read>
    where
        E: EntityTrait + Send + Sync + 'static,
        E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
        E::ActiveModel: Send + Sync + 'static,
    {
        Err(grouped_refused())
    }
}

fn grouped_refused() -> PyErr {
    ConstructionError::new_err("all_grouped reads a graph of one Opt slot, as Rust's does")
}

impl<F> RegisteredSlot for Opt<F>
where
    F: EntityTrait + Send + Sync + 'static,
    F::Model: IntoActiveModel<F::ActiveModel> + Sync + 'static,
    F::ActiveModel: Send + Sync + 'static,
{
    fn source(registry: &Registry) -> PyResult<Source> {
        Ok(Source {
            entity: registry.registered::<F>()?,
            optional: true,
        })
    }
    fn model(value: Self::Out, source: &Source) -> Option<PyEntityModel> {
        value.map(|value| PyEntityModel {
            inner: Arc::new(ModelAdapter::<F> {
                value,
                info: source.entity.clone(),
            }),
        })
    }
    fn grouped<E>(graph: SelectGraph<E, (Self,)>, bindings: &GraphBindings) -> PyResult<Read>
    where
        E: EntityTrait + Send + Sync + 'static,
        E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
        E::ActiveModel: Send + Sync + 'static,
    {
        Ok(grouped_by::<E, F>(graph, bindings))
    }
}

/// Native conversion machinery for Rust's existing graph tuple arities.
/// The pgorm Slots supertrait is sealed; arbitrary shapes cannot implement it.
pub trait GraphSlots<E: EntityTrait>:
    Slots + std::fmt::Debug + Send + Sync + Sized + 'static
where
    GraphRow<E, Self>: SelectorTrait,
{
    #[doc(hidden)]
    fn bindings(registry: &Registry) -> PyResult<GraphBindings>;
    #[doc(hidden)]
    fn models(row: GraphItem<E, Self>, bindings: &GraphBindings) -> Vec<Option<PyEntityModel>>;
    /// `cursor_by` over root columns for source 0, `cursor_by_on::<I, _>`
    /// over slot `I`'s otherwise.
    #[doc(hidden)]
    fn keyset(
        graph: &SelectGraph<E, Self>,
        source: usize,
        columns: &[String],
        bindings: &GraphBindings,
    ) -> PyResult<SlotKeyset>;
    #[doc(hidden)]
    fn grouped(graph: SelectGraph<E, Self>, bindings: &GraphBindings) -> PyResult<GroupedRead>;
}

fn no_source(source: usize) -> PyErr {
    ConstructionError::new_err(format!("the graph has no slot {source}"))
}

// [spec:pgorm:req:bindings.entity-graphs+4]
impl<E> GraphSlots<E> for ()
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn bindings(registry: &Registry) -> PyResult<GraphBindings> {
        Ok(GraphBindings {
            sources: vec![Source {
                entity: registry.registered::<E>()?,
                optional: false,
            }],
        })
    }
    fn models(value: E::Model, bindings: &GraphBindings) -> Vec<Option<PyEntityModel>> {
        vec![Some(PyEntityModel {
            inner: Arc::new(ModelAdapter::<E> {
                value,
                info: bindings.sources[0].entity.clone(),
            }),
        })]
    }
    fn keyset(
        graph: &SelectGraph<E, Self>,
        source: usize,
        columns: &[String],
        bindings: &GraphBindings,
    ) -> PyResult<SlotKeyset> {
        match source {
            0 => root_keyset(graph, columns, bindings).map(SlotKeyset),
            other => Err(no_source(other)),
        }
    }
    fn grouped(_: SelectGraph<E, Self>, _: &GraphBindings) -> PyResult<GroupedRead> {
        Err(grouped_refused())
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
            $($slot: RegisteredSlot + std::fmt::Debug + Send + Sync + 'static,)+
        {
            fn bindings(registry: &Registry) -> PyResult<GraphBindings> {
                Ok(GraphBindings { sources: vec![Source { entity: registry.registered::<E>()?, optional: false }, $($slot::source(registry)?,)+] })
            }
            fn models(row: GraphItem<E, Self>, bindings: &GraphBindings) -> Vec<Option<PyEntityModel>> {
                let root = PyEntityModel { inner: Arc::new(ModelAdapter::<E> { value: row.0, info: bindings.sources[0].entity.clone() }) };
                vec![Some(root), $($slot::model(row.$at, &bindings.sources[$at]),)+]
            }
            fn keyset(
                graph: &SelectGraph<E, Self>,
                source: usize,
                columns: &[String],
                bindings: &GraphBindings,
            ) -> PyResult<SlotKeyset> {
                match source {
                    0 => root_keyset(graph, columns, bindings).map(SlotKeyset),
                    $($at => slot_keyset::<$at, E, Self>(graph, columns, bindings).map(SlotKeyset),)+
                    other => Err(no_source(other)),
                }
            }
            fn grouped(graph: SelectGraph<E, Self>, bindings: &GraphBindings) -> PyResult<GroupedRead> {
                slots!(@grouped $grouped graph bindings $($slot)+)
            }
        }
    };
    (@grouped one $graph:ident $bindings:ident $slot:ident) => {
        <$slot as RegisteredSlot>::grouped($graph, $bindings).map(GroupedRead)
    };
    (@grouped many $graph:ident $bindings:ident $($slot:ident)+) => {{
        let _ = ($graph, $bindings);
        Err(grouped_refused())
    }};
}

slots!(one; S1 @ 1);
slots!(many; S1 @ 1, S2 @ 2);
slots!(many; S1 @ 1, S2 @ 2, S3 @ 3);
slots!(many; S1 @ 1, S2 @ 2, S3 @ 3, S4 @ 4);
slots!(many; S1 @ 1, S2 @ 2, S3 @ 3, S4 @ 4, S5 @ 5);
slots!(many; S1 @ 1, S2 @ 2, S3 @ 3, S4 @ 4, S5 @ 5, S6 @ 6);
