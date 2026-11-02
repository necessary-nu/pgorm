use pgorm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Entity, Serialize, Deserialize)]
#[pgorm(table_name = "notes", schema_name = "fixture")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub account_id: i32,
    pub tenant: i32,
    pub body: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
