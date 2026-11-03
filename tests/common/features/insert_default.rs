use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "insert_default")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
