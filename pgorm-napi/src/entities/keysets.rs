//! A registered graph's keyset cursors, `SelectGraph::cursor_by` over root
//! columns and `cursor_by_on::<I, _>` over a slot's, and its grouped read.
//! Each order key is a tuple of the entity's own columns, whose arity is the
//! cursor's: a key of one to six columns is built here, one arm per arity.

use std::sync::Arc;

use pgorm::{
    Cursor, EntityTrait, GraphItem, GraphRow, IntoActiveModel, Opt, SelectGraph, SelectorTrait,
    Slot, SlotAt,
};

use super::{
    adapter::{column, model_of},
    graph::{GraphBindings, GraphSlots, GroupedFn},
    reads::{KeyColumn, Keyed, Keyset, Row},
};

fn keyed<S, K>(
    cursor: Box<dyn Fn() -> Cursor<S, K> + Send + Sync>,
    decode: Arc<dyn Fn(S::Item) -> Row + Send + Sync>,
    order: Vec<KeyColumn>,
    key: Vec<KeyColumn>,
    bindings: &GraphBindings,
) -> Arc<dyn Keyset>
where
    S: SelectorTrait + Send + Sync + 'static,
    S::Item: Send,
    K: Send + Sync + 'static,
{
    Arc::new(Keyed {
        cursor,
        decode,
        order,
        key,
        sources: bindings
            .sources
            .iter()
            .map(|source| source.entity.clone())
            .collect(),
    })
}

/// The keyset `$make` builds of `$graph` and a key of `$columns`, one arm
/// per arity since each arity is its own key type.
macro_rules! by_arity {
    ($graph:ident, $bindings:ident, $columns:ident, $decode:ident, $order:ident, $key:ident, |$g:ident, $k:ident| $make:expr) => {
        match $columns.as_slice() {
            [a] => {
                let (graph, a) = ($graph.clone(), *a);
                Ok(keyed(
                    Box::new(move || {
                        let ($g, $k) = (graph.clone(), a);
                        $make
                    }),
                    $decode,
                    $order,
                    $key,
                    $bindings,
                ))
            }
            [a, b] => {
                let (graph, key) = ($graph.clone(), (*a, *b));
                Ok(keyed(
                    Box::new(move || {
                        let ($g, $k) = (graph.clone(), key);
                        $make
                    }),
                    $decode,
                    $order,
                    $key,
                    $bindings,
                ))
            }
            [a, b, c] => {
                let (graph, key) = ($graph.clone(), (*a, *b, *c));
                Ok(keyed(
                    Box::new(move || {
                        let ($g, $k) = (graph.clone(), key);
                        $make
                    }),
                    $decode,
                    $order,
                    $key,
                    $bindings,
                ))
            }
            [a, b, c, d] => {
                let (graph, key) = ($graph.clone(), (*a, *b, *c, *d));
                Ok(keyed(
                    Box::new(move || {
                        let ($g, $k) = (graph.clone(), key);
                        $make
                    }),
                    $decode,
                    $order,
                    $key,
                    $bindings,
                ))
            }
            [a, b, c, d, e] => {
                let (graph, key) = ($graph.clone(), (*a, *b, *c, *d, *e));
                Ok(keyed(
                    Box::new(move || {
                        let ($g, $k) = (graph.clone(), key);
                        $make
                    }),
                    $decode,
                    $order,
                    $key,
                    $bindings,
                ))
            }
            [a, b, c, d, e, f] => {
                let (graph, key) = ($graph.clone(), (*a, *b, *c, *d, *e, *f));
                Ok(keyed(
                    Box::new(move || {
                        let ($g, $k) = (graph.clone(), key);
                        $make
                    }),
                    $decode,
                    $order,
                    $key,
                    $bindings,
                ))
            }
            [] => Err("a cursor orders by at least one column".to_owned()),
            _ => Err("a registered graph's cursor orders by at most six columns".to_owned()),
        }
    };
}

/// Every source's key column, the root's and then each slot's in order, as
/// `(source, column)`.
fn primary_keys(bindings: &GraphBindings) -> Vec<(usize, KeyColumn)> {
    bindings
        .sources
        .iter()
        .enumerate()
        .flat_map(|(index, source)| {
            source.entity.primary_key.iter().map(move |name| {
                (
                    index,
                    KeyColumn {
                        entity: source.entity.clone(),
                        column: name.clone(),
                    },
                )
            })
        })
        .collect()
}

/// The order columns of source `ordered`, and the whole key a cursor over
/// them compares: those columns, then the tiebreaks pgorm installs — the
/// ordered source's own key first, the other sources' after it in source
/// order — less any column of the ordered source already ordered by.
fn key_of(
    bindings: &GraphBindings,
    ordered: usize,
    columns: &[String],
) -> (Vec<KeyColumn>, Vec<KeyColumn>) {
    let entity = &bindings.sources[ordered].entity;
    let order: Vec<_> = columns
        .iter()
        .map(|name| KeyColumn {
            entity: entity.clone(),
            column: name.clone(),
        })
        .collect();
    let keys = primary_keys(bindings);
    let own = keys.iter().filter(|(source, _)| *source == ordered);
    let others = keys.iter().filter(|(source, _)| *source != ordered);
    let key = order
        .iter()
        .cloned()
        .chain(
            own.filter(|(_, column)| !columns.contains(&column.column))
                .chain(others)
                .map(|(_, column)| column.clone()),
        )
        .collect();
    (order, key)
}

fn decoder<E, S>(bindings: &GraphBindings) -> Arc<dyn Fn(GraphItem<E, S>) -> Row + Send + Sync>
where
    E: EntityTrait + Send + Sync + 'static,
    S: GraphSlots<E>,
    GraphRow<E, S>: SelectorTrait,
{
    let bindings = bindings.clone();
    Arc::new(move |item| {
        S::models(item, &bindings)
            .into_iter()
            .map(|model| model.map(|model| model.0))
            .collect()
    })
}

// [spec:pgorm:req:bindings.entity-graphs+4]
pub(super) fn root_keyset<E, S>(
    graph: &SelectGraph<E, S>,
    columns: &[String],
    bindings: &GraphBindings,
) -> Result<Arc<dyn Keyset>, String>
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
    let (order, key) = key_of(bindings, 0, columns);
    let decode = decoder::<E, S>(bindings);
    by_arity!(graph, bindings, resolved, decode, order, key, |g, k| g
        .cursor_by(k))
}

// [spec:pgorm:req:bindings.entity-graphs+4]
pub(super) fn slot_keyset<const I: usize, E, S>(
    graph: &SelectGraph<E, S>,
    columns: &[String],
    bindings: &GraphBindings,
) -> Result<Arc<dyn Keyset>, String>
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
    let (order, key) = key_of(bindings, I, columns);
    let decode = decoder::<E, S>(bindings);
    by_arity!(graph, bindings, resolved, decode, order, key, |g, k| g
        .cursor_by_on::<I, _>(k))
}

/// `all_grouped` of a graph of one `Opt` slot: each root with its slot's
/// models, in the order roots first appear.
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
    Box::new(move |db| {
        Box::pin(async move {
            let groups = graph.all_grouped(&db).await?;
            Ok(groups
                .into_iter()
                .map(|(parent, children)| {
                    (
                        model_of::<E>(parent, &root),
                        children
                            .into_iter()
                            .map(|child| model_of::<F>(child, &slot))
                            .collect(),
                    )
                })
                .collect())
        })
    })
}
