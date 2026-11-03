use crate as pgorm;
use crate::entity::prelude::*;

/// The kind `truck` of [`Vehicle`](super::vehicle::Vehicle), keyed by its
/// vehicle's `id` under a name of its own.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "truck")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub vehicle_id: i32,
    pub axles: i16,
    pub payload_kg: i32,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
