//! A registered `Select<E>`'s pages and keyset cursor, through pgorm's own
//! `PaginatorTrait` and `Select::cursor_by`.

use std::sync::Arc;

use pgorm::{
    EntityTrait, Error, IntoActiveModel,
    pgorm_query::{Key, Name},
};

use super::{
    generic::{SelectAdapter, column, model_of},
    reads::{KeyColumn, Keyed, Keyset, Paged, Pages, one_source},
};

/// The `Select`'s pages, each item its entity's model.
pub(super) fn pages<E>(select: &SelectAdapter<E>) -> Arc<dyn Pages>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    let info = select.info.clone();
    Arc::new(Paged {
        query: select.query.clone(),
        decode: Arc::new(move |value| one_source(model_of::<E>(value, &info))),
    })
}

/// `Select::cursor_by` over `columns`, a key whose arity is the list's: a
/// plain read has no tiebreak, so its whole key is its order columns.
// [spec:pgorm:req:bindings.entity-reads+4]
pub(super) fn cursor<E>(
    select: &SelectAdapter<E>,
    columns: &[String],
) -> Result<Arc<dyn Keyset>, Error>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    if columns.is_empty() {
        return Err(Error::Type("a cursor orders by at least one column".into()));
    }
    let names = columns
        .iter()
        .map(|name| column::<E>(name).map(|_| Name::runtime(name)))
        .collect::<Result<Vec<_>, _>>()?;
    let order: Vec<_> = columns
        .iter()
        .map(|name| KeyColumn {
            entity: select.info.clone(),
            column: name.clone(),
        })
        .collect();
    let info = select.info.clone();
    let query = select.query.clone();
    Ok(Arc::new(Keyed {
        cursor: Box::new(move || query.clone().cursor_by(Key::from(names.clone()))),
        decode: Arc::new(move |value| one_source(model_of::<E>(value, &info))),
        key: order.clone(),
        order,
    }))
}
