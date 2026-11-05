use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "parent")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id1: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub id2: i32,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::child::Entity)]
    Child,
}
