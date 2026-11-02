use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "tenant_ticket")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub tenant_id: i32,
    #[pgorm(primary_key, identity)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::Text)]
    pub title: String,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
