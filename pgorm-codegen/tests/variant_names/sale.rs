use pgorm::entity::prelude::*;

use super::pgorm_active_enums::Outlet;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "sale")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub outlet: Outlet,
    pub web_shop_id: Option<i32>,
    pub pop_up_shop_id: Option<i32>,
    pub closed_shop_id: Option<i32>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::closed_shop::Entity,
        from = Column::ClosedShopId,
        to = super::closed_shop::Column::Id
    )]
    ClosedShop,
    #[pgorm(
        belongs_to = super::pop_up_shop::Entity,
        from = Column::PopUpShopId,
        to = super::pop_up_shop::Column::Id
    )]
    PopUpShop,
    #[pgorm(
        belongs_to = super::web_shop::Entity,
        from = Column::WebShopId,
        to = super::web_shop::Column::Id
    )]
    WebShop,
}
