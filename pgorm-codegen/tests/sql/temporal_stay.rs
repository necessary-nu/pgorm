use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "temporal_stay")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub room_id: i32,
    pub during: Range<i32>,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::temporal_room::Entity,
        from = Column::RoomId,
        to = super::temporal_room::Column::Id,
        from_period = Column::During,
        to_period = super::temporal_room::Column::ValidAt
    )]
    TemporalRoom,
}
