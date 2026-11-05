use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "topic")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub title: String,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::remark::Entity)]
    Remark,
}
