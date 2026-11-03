use crate as pgorm;
use crate::entity::prelude::*;

/// The kind `storefront` of [`Shop`](super::shop::Shop), keyed by its shop's
/// `id`.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "storefront")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub street: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::shop::Entity,
        from = Column::Id,
        to = super::shop::Column::Id
    )]
    Shop,
}
