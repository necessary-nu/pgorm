use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "pop_up_shop")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub site: String,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::sale::Entity)]
    Sale,
}
