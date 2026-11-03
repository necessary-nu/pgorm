use pgorm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Entity, Eq, Serialize, Deserialize)]
#[pgorm(table_name = "truck")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub vehicle_id: i32,
    pub axles: i16,
    pub payload_kg: i32,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::vehicle::Entity,
        from = Column::VehicleId,
        to = super::vehicle::Column::Id
    )]
    Vehicle,
}
