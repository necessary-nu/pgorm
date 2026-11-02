use crate as pgorm;
use crate::entity::prelude::*;

#[cfg(feature = "with-json")]
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[cfg_attr(feature = "with-json", derive(Serialize, Deserialize))]
#[pgorm(table_name = "fruit")]
pub struct Model {
    #[pgorm(primary_key)]
    #[cfg_attr(feature = "with-json", serde(skip_deserializing))]
    pub id: i32,
    pub name: String,
    pub cake_id: Option<i32>,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::cake::Entity,
        from = Column::CakeId,
        to = super::cake::Column::Id
    )]
    Cake,
}
