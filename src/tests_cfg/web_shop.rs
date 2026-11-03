use crate as pgorm;
use crate::entity::prelude::*;

/// The kind `web_shop` of [`Shop`](super::shop::Shop), keyed by its shop's
/// `id` under a name of its own.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "web_shop")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub shop_id: i32,
    pub url: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::shop::Entity,
        from = Column::ShopId,
        to = super::shop::Column::Id
    )]
    Shop,
}
