use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "temporal_room")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    #[pgorm(primary_key, without_overlaps)]
    pub valid_at: Range<i32>,
    pub rate: i32,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::temporal_stay::Entity)]
    TemporalStay,
}
