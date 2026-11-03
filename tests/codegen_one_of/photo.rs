use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "photo")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i64,
    pub url: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::comment::Entity)]
    Comment,
}
