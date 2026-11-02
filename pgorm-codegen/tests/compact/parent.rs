use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "parent")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id1: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub id2: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(has_many = "super::child::Entity")]
    Child,
}

impl Related<super::child::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Child.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
