use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "byte_primary_key")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: Vec<u8>,
    pub value: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
