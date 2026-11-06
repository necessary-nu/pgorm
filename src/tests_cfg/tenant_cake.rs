//! A cake table kept once per tenant, each in a schema named for its tenant:
//! an entity whose names are computed at run time.

use crate as pgorm;
use crate::entity::prelude::*;

/// The entity holds what its names are computed from. Statements over
/// `Entity::default()` read the `public` tenant; the `_in` constructors read
/// the tenant of the instance they are given.
#[derive(Copy, Clone, Debug, EntityTrait)]
pub struct Entity {
    pub tenant: &'static str,
}

impl Default for Entity {
    fn default() -> Self {
        Entity { tenant: "public" }
    }
}

impl EntityName for Entity {
    fn schema_name(&self) -> Option<&str> {
        Some(self.tenant)
    }

    fn table_name(&self) -> &str {
        "cake"
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
