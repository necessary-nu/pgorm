use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(schema_name = "market", table_name = "flat")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub tenant: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub listing: i64,
    pub rooms: i16,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::listing::Entity,
        from = (Column::Tenant, Column::Listing),
        to = (super::listing::Column::TenantId, super::listing::Column::Id)
    )]
    Listing,
}
