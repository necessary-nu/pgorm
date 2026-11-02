use super::pgorm_active_enums::*;
use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "active_enum_child")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub parent_id: i32,
    pub category: Option<Category>,
    pub color: Option<Color>,
    pub tea: Option<Tea>,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(
        fk_name = "fk-active_enum_child-active_enum",
        belongs_to = "super::active_enum::Entity",
        from = "Column::ParentId",
        to = "super::active_enum::Column::Id"
    )]
    ActiveEnum,
}

impl Related<super::active_enum::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ActiveEnum.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
