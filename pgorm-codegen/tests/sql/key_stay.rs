use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "key_stay")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub room_id: i32,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::key_room::Entity,
        from = Column::RoomId,
        to = super::key_room::Column::Id,
        deferrability = Deferrability::DeferrableInitiallyDeferred,
        enforcement = Enforcement::NotEnforced
    )]
    KeyRoom,
}
