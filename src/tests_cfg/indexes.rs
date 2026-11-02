//! An entity definition for testing table index creation.
use crate as pgorm;
use crate::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(schema_name = "public", table_name = "indexes")]
pub struct Model {
    #[pgorm(primary_key)]
    pub indexes_id: i32,
    #[pgorm(unique)]
    pub unique_attr: i32,
    #[pgorm(indexed)]
    pub index1_attr: i32,
    #[pgorm(indexed, unique)]
    pub index2_attr: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
