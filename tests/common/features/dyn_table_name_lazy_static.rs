use arraystring::{ArrayString, typenum::U32};
use pgorm::entity::prelude::*;

pub type TableName = ArrayString<U32>;

/// A table name chosen per statement. Its columns are qualified by the
/// default's name, so a statement over another instance reads its table
/// under that name.
#[derive(Copy, Clone, Debug, EntityTrait)]
pub struct Entity {
    pub table_name: TableName,
}

impl Default for Entity {
    fn default() -> Self {
        Entity {
            table_name: TableName::from_str_truncate("dyn_table_name_lazy_static"),
        }
    }
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
