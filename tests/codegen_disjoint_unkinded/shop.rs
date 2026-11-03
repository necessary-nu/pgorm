use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "shop")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub name: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_one = super::storefront::Entity)]
    Storefront,
    #[pgorm(has_one = super::website::Entity)]
    Website,
}

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = Entity, unkinded)]
pub enum Shop {
    Website(super::website::Model),
    Storefront(super::storefront::Model),
    Dormant,
}
