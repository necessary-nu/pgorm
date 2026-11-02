use pgorm::entity::prelude::*;

/// A room's rate over time, each rate a version of the room keyed by the room
/// and the period it holds over: `PRIMARY KEY (id, valid_at WITHOUT
/// OVERLAPS)`, registered so that Python reaches a temporal key.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "rooms", schema_name = "python_entities")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    #[pgorm(primary_key, without_overlaps)]
    pub valid_at: Range<Date>,
    pub rate: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
