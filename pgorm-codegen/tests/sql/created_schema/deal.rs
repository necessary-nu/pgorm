use pgorm::entity::prelude::*;

use super::pgorm_active_enums::Stage;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(schema_name = "sales", table_name = "deal")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::Text)]
    pub title: String,
    pub stage: Stage,
    pub history: Option<Vec<Stage>>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
