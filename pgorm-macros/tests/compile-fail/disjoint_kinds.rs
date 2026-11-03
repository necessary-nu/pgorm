use pgorm::entity::prelude::*;
use pgorm::tests_cfg::{car, truck};

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::vehicle::Entity)]
pub enum NamedFields {
    Car { model: car::Model },
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::vehicle::Entity)]
pub enum TwoFields {
    Car(car::Model, truck::Model),
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::vehicle::Entity)]
pub enum NoField {
    Car(),
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::vehicle::Entity)]
pub enum Discriminated {
    Bike = 1,
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::vehicle::Entity)]
pub enum UnknownVariantKey {
    #[pgorm(string_value = "bike")]
    Bike,
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::vehicle::Entity)]
pub enum PayloadKey {
    Car(#[pgorm(ignore)] car::Model),
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::vehicle::Entity)]
pub enum SameValue {
    Car(car::Model),
    #[pgorm(value = "car")]
    Truck(truck::Model),
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::vehicle::Entity)]
pub enum SameDefault {
    #[pgorm(value = "pickup_truck")]
    Truck(truck::Model),
    PickupTruck,
}

fn main() {}
