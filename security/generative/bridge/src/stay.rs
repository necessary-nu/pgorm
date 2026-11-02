use pgorm::entity::prelude::*;

/// A guest's stay in a room: its room a temporal foreign key checked at
/// commit, its guest an account the server never checks.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "stays", schema_name = "fixture")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub room_id: i32,
    pub guest_id: i32,
    pub during: Range<Date>,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
