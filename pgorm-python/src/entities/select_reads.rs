//! A registered `Select<E>`'s pages and keyset cursor, pgorm's erased
//! paginator and `select_cursor`.

use std::sync::Arc;

use pgorm::{EntityTrait, Error, IntoActiveModel, PagedRead, pgorm_query::Name, select_cursor};

use super::{
    generic::{SelectAdapter, column, model_of},
    reads::{KeyColumn, Keyset, Pages, one_source},
};

/// The `Select`'s pages, each item its entity's model, through pgorm's erased
/// paginator.
pub(super) fn pages<E>(select: &SelectAdapter<E>) -> Arc<Pages>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    let info = select.info.clone();
    Arc::new(PagedRead::new(select.query.clone(), move |value| {
        one_source(model_of::<E>(value, &info))
    }))
}

/// `Select::cursor_by` over `columns`, pgorm's `select_cursor`: a plain read
/// has no tiebreak, so its whole key is its order columns.
// [spec:pgorm:req:bindings.entity-reads+4]
pub(super) fn cursor<E>(select: &SelectAdapter<E>, columns: &[String]) -> Result<Arc<Keyset>, Error>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    let names = columns
        .iter()
        .map(|name| column::<E>(name).map(|_| Name::runtime(name)))
        .collect::<Result<Vec<_>, _>>()?;
    let info = select.info.clone();
    let cursor = select_cursor(select.query.clone(), names, move |value| {
        one_source(model_of::<E>(value, &info))
    })?;
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
    }))
}
