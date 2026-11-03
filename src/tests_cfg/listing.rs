use crate as pgorm;
use crate::entity::prelude::*;

/// The base of the disjoint [`Listing`]: keyed by two columns, its kind
/// column under a name of its own, in the schema `market`.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "listing", schema_name = "market")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub tenant_id: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i64,
    pub category: ListingKind,
    pub title: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}

/// A listing by its kind, its kind type named and placed by hand: an
/// apartment has a row of its own, a building plot none.
// [spec:pgorm:def:disjoint.declare+5]
#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(
    base = Entity,
    kind = Column::Category,
    enum_name = "listing_category",
    schema_name = "market"
)]
pub enum Listing {
    #[pgorm(value = "apartment")]
    Flat(super::flat::Model),
    BuildingPlot,
}
