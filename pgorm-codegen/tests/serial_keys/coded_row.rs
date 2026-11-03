use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "coded_row")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false, column_type = ColumnType::Text)]
    pub code: String,
    pub seq: i32,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
