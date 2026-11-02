use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "edit_log")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub action: String,
    pub values: Json,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
