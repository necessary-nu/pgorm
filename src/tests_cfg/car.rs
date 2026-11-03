use crate as pgorm;
use crate::entity::prelude::*;

/// The kind `car` of [`Vehicle`](super::vehicle::Vehicle), keyed by its
/// vehicle's `id`, which it declares a relation over as well.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "car")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub seats: i16,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::vehicle::Entity,
        from = Column::Id,
        to = super::vehicle::Column::Id
    )]
    Vehicle,
}
