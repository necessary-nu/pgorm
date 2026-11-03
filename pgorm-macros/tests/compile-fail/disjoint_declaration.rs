use pgorm::entity::prelude::*;
use pgorm::tests_cfg::car;

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::vehicle::Entity)]
pub struct NotAnEnum {
    pub car: car::Model,
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::vehicle::Entity)]
pub enum Generic<T> {
    Car(car::Model),
    Other(T),
}

#[derive(Disjoint)]
pub enum NoBase {
    Car(car::Model),
}

#[derive(Disjoint)]
#[pgorm(base = "pgorm::tests_cfg::vehicle::Entity")]
pub enum QuotedBase {
    Car(car::Model),
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::vehicle::Entity, base = pgorm::tests_cfg::vehicle::Entity)]
pub enum BaseTwice {
    Car(car::Model),
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::vehicle::Entity, kind_column = pgorm::tests_cfg::vehicle::Column::Kind)]
pub enum UnknownKey {
    Car(car::Model),
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::vehicle::Entity)]
pub enum NoKinds {}

fn main() {}
