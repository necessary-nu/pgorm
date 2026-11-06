//! A note kept once per tenant, each tenant's in a schema of its own: an
//! entity whose names are computed at run time, registered once per tenant.

use pgorm::entity::prelude::*;

#[derive(Copy, Clone, Debug, EntityTrait)]
pub struct Entity {
    pub tenant: &'static str,
}

impl Default for Entity {
    fn default() -> Self {
        Self {
            tenant: "napi_entities",
        }
    }
}

impl EntityName for Entity {
    fn schema_name(&self) -> Option<&str> {
        Some(self.tenant)
    }

    fn table_name(&self) -> &str {
        "tenant_notes"
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub body: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
