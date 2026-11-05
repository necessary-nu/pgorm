use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(schema_name = "market", table_name = "listing")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub tenant_id: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i64,
    pub category: ListingKind,
    pub title: String,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_one = super::flat::Entity)]
    Flat,
}

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(
    base = Entity,
    kind = Column::Category,
    enum_name = "listing_category",
    schema_name = "market"
)]
pub enum Listing {
    Apartment(super::flat::Model),
    BuildingPlot,
}
