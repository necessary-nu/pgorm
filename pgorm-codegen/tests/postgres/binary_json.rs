use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "task")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub payload: Json,
    #[pgorm(column_type = ColumnType::JsonBinary)]
    pub payload_binary: Json,
}
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
