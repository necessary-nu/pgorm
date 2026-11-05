use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "store")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub name: String,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_one = super::pop_up_store::Entity)]
    PopUpStore,
    #[pgorm(has_one = super::web_store::Entity)]
    WebStore,
}

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = Entity, unkinded)]
pub enum Store {
    WebStore(super::web_store::Model),
    PopUpStore(super::pop_up_store::Model),
    ClosedStore,
}
