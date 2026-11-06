//! The reads a registered `Select<E>` and `SelectGraph<E, S>` have beside
//! `all` and `one_opt`: their pages, pgorm's erased paginator; their keyset
//! cursor, pgorm's erased cursor; and their rows decoded by the read's own
//! selector as each arrives.

use std::sync::Arc;

use pgorm::{
    EntityTrait, ErasedCursor, ErasedPages, Error, IntoActiveModel, PagedRead, QueryResult,
    pgorm_query::Name, select_cursor,
};

use super::{
    EntityInfo, Model,
    adapter::{SelectAdapter, column, model_of},
};
use crate::{Failure, Metrics};

/// A row of a registered read: each source's model, `None` for an absent
/// optional slot.
pub type Row = Vec<Option<Model>>;

/// The decode of one row of a registered read's result into its sources'
/// models, as its selector's `from_raw_query_result` reads it.
pub type Decode = Arc<dyn Fn(QueryResult) -> Result<Row, Error> + Send + Sync>;

/// A registered read's pages: pgorm's erased paginator, and the entities its
/// rows' sources decode as.
#[derive(Debug)]
#[allow(missing_docs)]
pub struct Pages {
    pub pages: Box<dyn ErasedPages<Row, Metrics>>,
    pub sources: Vec<Arc<EntityInfo>>,
}

/// A column a cursor's boundary value converts through: its entity, and its
/// name there.
#[derive(Clone, Debug)]
#[allow(missing_docs)]
pub struct KeyColumn {
    pub entity: Arc<EntityInfo>,
    pub column: String,
}

/// A registered read's keyset cursor: pgorm's erased cursor, its order
/// columns, the whole key a `_with` boundary names, and the entities its
/// rows' sources decode as.
#[derive(Debug)]
#[allow(missing_docs)]
pub struct Keyset {
    pub cursor: Arc<dyn ErasedCursor<Row, Metrics>>,
    pub order: Vec<KeyColumn>,
    /// The whole key a `_with` boundary names: the order columns, and the
    /// tiebreak a graph completes them with.
    pub key: Vec<KeyColumn>,
    pub sources: Vec<Arc<EntityInfo>>,
}

/// The `Select`'s pages, each item its entity's model, through pgorm's erased
/// paginator.
pub(super) fn pages<E>(select: &SelectAdapter<E>) -> Arc<Pages>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    let info = select.info.clone();
    Arc::new(Pages {
        pages: Box::new(PagedRead::new(select.query.clone(), move |value| {
            vec![Some(model_of::<E>(value, &info))]
        })),
        sources: vec![select.info.clone()],
    })
}

/// `Select::cursor_by` over `columns`, pgorm's `select_cursor`: a plain read
/// has no tiebreak, so its whole key is its order columns.
// [spec:pgorm:req:bindings.entity-reads+4]
pub(super) fn cursor<E>(
    select: &SelectAdapter<E>,
    columns: &[String],
) -> Result<Arc<Keyset>, Failure>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    let names = columns
        .iter()
        .map(|name| column::<E>(&select.info, name).map(|_| Name::runtime(name)))
        .collect::<Result<Vec<_>, _>>()?;
    let info = select.info.clone();
    let cursor = select_cursor(select.query.clone(), names, move |value| {
        vec![Some(model_of::<E>(value, &info))]
    })
    .map_err(|arity| Failure::Construction(arity.to_string()))?;
    let order: Vec<_> = columns
        .iter()
        .map(|name| KeyColumn {
            entity: select.info.clone(),
            column: name.clone(),
        })
        .collect();
    Ok(Arc::new(Keyset {
        cursor,
        key: order.clone(),
        order,
        sources: vec![select.info.clone()],
    }))
}
