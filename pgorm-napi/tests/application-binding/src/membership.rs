//! An account's membership of a team, keyed by both.

use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "memberships", schema_name = "napi_entities")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub account_id: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub team: String,
    pub role: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
