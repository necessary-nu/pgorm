use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "badge")]
pub struct Model {
    #[pgorm(primary_key, identity)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::Text)]
    pub label: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
