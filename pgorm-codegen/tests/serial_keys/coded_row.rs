use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "coded_row")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false, column_type = ColumnType::Text)]
    pub code: String,
    pub seq: i32,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
