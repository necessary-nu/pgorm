use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "web_shop")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub url: String,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::sale::Entity)]
    Sale,
}
