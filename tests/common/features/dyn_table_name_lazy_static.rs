use arraystring::{ArrayString, typenum::U32};
use pgorm::entity::prelude::*;

pub type TableName = ArrayString<U32>;

#[derive(Copy, Clone, Default, Debug, EntityTrait)]
pub struct Entity {
    pub table_name: TableName,
}

impl EntityName for Entity {
    fn table_name(&self) -> &str {
        self.table_name.as_str()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub name: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
