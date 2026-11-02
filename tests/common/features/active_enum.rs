use super::pgorm_active_enums::*;
use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "active_enum")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub category: Option<Category>,
    pub color: Option<Color>,
    pub tea: Option<Tea>,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::active_enum_child::Entity)]
    ActiveEnumChild,
}
