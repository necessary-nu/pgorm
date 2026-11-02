use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "users")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::Text)]
    pub email: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::bills::Entity)]
    Bills,
}
