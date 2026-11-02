use pgorm::entity::prelude::*;

/// A guest's stay in a room. Its room is a temporal foreign key, every version
/// of the room whose period overlaps the stay, checked at commit; its guest is
/// an account the server records and never checks, so a stay may name one
/// that does not exist.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "stays", schema_name = "python_entities")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub room_id: i32,
    pub guest_id: i32,
    pub during: Range<Date>,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::room::Entity,
        from = Column::RoomId,
        to = super::room::Column::Id,
        from_period = Column::During,
        to_period = super::room::Column::ValidAt,
        deferrability = Deferrability::DeferrableInitiallyDeferred
    )]
    Room,
    #[pgorm(
        belongs_to = super::account::Entity,
        from = Column::GuestId,
        to = super::account::Column::Id,
        enforcement = Enforcement::NotEnforced
    )]
    Guest,
}
