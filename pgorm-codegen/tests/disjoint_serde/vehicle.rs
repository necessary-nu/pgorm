use pgorm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Entity, Eq, Serialize, Deserialize)]
#[pgorm(table_name = "vehicle")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub kind: VehicleKind,
    pub name: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_one = super::car::Entity)]
    Car,
    #[pgorm(has_one = super::truck::Entity)]
    Truck,
}

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Debug, PartialEq, Eq, Disjoint, Serialize, Deserialize)]
#[pgorm(base = Entity, kind_attrs(derive(Serialize, Deserialize)))]
pub enum Vehicle {
    Car(super::car::Model),
    Truck(super::truck::Model),
    Bike,
}
