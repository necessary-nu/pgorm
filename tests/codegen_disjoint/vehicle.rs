use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "vehicle")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub kind: VehicleKind,
    pub name: String,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_one = super::car::Entity)]
    Car,
    #[pgorm(has_one = super::truck::Entity)]
    Truck,
}

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = Entity)]
pub enum Vehicle {
    Car(super::car::Model),
    Truck(super::truck::Model),
    Bike,
}
