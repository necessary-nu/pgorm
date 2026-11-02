use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "key_stay")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub room_id: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = "super::key_room::Entity",
        from = "Column::RoomId",
        to = "super::key_room::Column::Id",
        deferrability = "DeferrableInitiallyDeferred",
        enforcement = "NotEnforced"
    )]
    KeyRoom,
}

impl Related<super::key_room::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::KeyRoom.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
