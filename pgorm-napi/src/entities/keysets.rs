//! A registered graph's keyset cursors, `SelectGraph::cursor_by` over root
//! columns and `cursor_by_on::<I, _>` over a slot's, and its grouped read,
//! each pgorm's erased form of them; this module resolves the columns by
//! name and says what each key column converts a boundary value through.

use std::sync::Arc;

use pgorm::{
    EntityTrait, GraphItem, GraphRow, IntoActiveModel, Opt, SelectGraph, SelectorTrait, Slot,
    SlotAt, graph_cursor, graph_cursor_key, graph_cursor_on, grouped_read,
};

use super::{
    adapter::{column, model_of},
    graph::{GraphBindings, GraphSlots, GroupedFn},
    reads::{KeyColumn, Keyset, Row},
};

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
    move |item| {
        S::models(item, &bindings)
            .into_iter()
            .map(|model| model.map(|model| model.0))
            .collect()
    }
}

fn keyset(
    bindings: &GraphBindings,
    ordered: usize,
    columns: &[String],
    cursor: Arc<dyn pgorm::ErasedCursor<Row, pgorm_bindings::Metrics>>,
) -> Arc<Keyset> {
    let (order, key) = key_of(bindings, ordered, columns);
    Arc::new(Keyset {
        cursor,
        order,
        key,
        sources: bindings
            .sources
            .iter()
            .map(|source| source.entity.clone())
            .collect(),
    })
}

// [spec:pgorm:req:bindings.entity-graphs+4]
pub(super) fn root_keyset<E, S>(
    graph: &SelectGraph<E, S>,
    columns: &[String],
    bindings: &GraphBindings,
) -> Result<Arc<Keyset>, String>
where
    E: EntityTrait + Send + Sync + 'static,
    S: GraphSlots<E>,
    GraphRow<E, S>: SelectorTrait + Send + Sync + 'static,
    GraphItem<E, S>: Send,
{
    let resolved = columns
        .iter()
        .map(|name| column::<E>(&bindings.sources[0].entity, name))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|failure| failure.to_string())?;
    let cursor = graph_cursor(graph, &resolved, decoder::<E, S>(bindings))
        .map_err(|arity| arity.to_string())?;
    Ok(keyset(bindings, 0, columns, cursor))
}

// [spec:pgorm:req:bindings.entity-graphs+4]
pub(super) fn slot_keyset<const I: usize, E, S>(
    graph: &SelectGraph<E, S>,
    columns: &[String],
    bindings: &GraphBindings,
) -> Result<Arc<Keyset>, String>
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
        .collect::<Result<Vec<_>, _>>()
        .map_err(|failure| failure.to_string())?;
    let cursor = graph_cursor_on::<I, _, _, _, _>(graph, &resolved, decoder::<E, S>(bindings))
        .map_err(|arity| arity.to_string())?;
    Ok(keyset(bindings, I, columns, cursor))
}

/// `all_grouped` of a graph of one `Opt` slot, pgorm's `grouped_read`: each
/// root with its slot's models, in the order roots first appear.
// [spec:pgorm:req:bindings.entity-graphs+4]
pub(super) fn grouped_by<E, F>(
    graph: SelectGraph<E, (Opt<F>,)>,
    bindings: &GraphBindings,
) -> GroupedFn
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
