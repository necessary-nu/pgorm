use super::pgorm_active_enums::*;
use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "teas")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: Tea,
    pub category: Option<Category>,
    pub color: Option<Color>,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
