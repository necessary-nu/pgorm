use pgorm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// A membership keyed `(tenant_id, id)`, the database generating `id` unless a
/// row supplies one: the multi-tenant composite key, registered so that Python
/// reaches a Rust entity whose key is two columns wide.
#[derive(Clone, Debug, PartialEq, Eq, Entity, Serialize, Deserialize)]
#[pgorm(table_name = "memberships", schema_name = "python_entities")]
pub struct Model {
    #[pgorm(primary_key)]
    pub tenant_id: i32,
    #[pgorm(primary_key, identity_by_default)]
    pub id: i32,
    pub role: String,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
