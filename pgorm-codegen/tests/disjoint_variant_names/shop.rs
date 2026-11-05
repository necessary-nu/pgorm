use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "shop")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub kind: ShopKind,
    pub name: String,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_one = super::pop_up_shop::Entity)]
    PopUpShop,
    #[pgorm(has_one = super::web_shop::Entity)]
    WebShop,
}

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = Entity)]
pub enum Shop {
    WebShop(super::web_shop::Model),
    PopUpShop(super::pop_up_shop::Model),
    ClosedShop,
}
