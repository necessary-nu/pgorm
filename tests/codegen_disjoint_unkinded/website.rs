use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "website")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub shop_id: i32,
    pub url: String,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::shop::Entity,
        from = Column::ShopId,
        to = super::shop::Column::Id
    )]
    Shop,
}
