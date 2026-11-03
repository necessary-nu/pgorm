use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "tenant_order")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub tenant_id: i32,
    #[pgorm(primary_key)]
    pub id: i32,
    pub total: i32,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
