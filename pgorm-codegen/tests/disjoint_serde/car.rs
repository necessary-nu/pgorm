use pgorm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Entity, Eq, Serialize, Deserialize)]
#[pgorm(table_name = "car")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub seats: i16,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::vehicle::Entity,
        from = Column::Id,
        to = super::vehicle::Column::Id
    )]
    Vehicle,
}
