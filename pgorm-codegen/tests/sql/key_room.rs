use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "key_room")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::key_stay::Entity)]
    KeyStay,
}

impl Related<super::key_stay::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::KeyStay.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
