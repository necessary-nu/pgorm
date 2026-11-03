use crate as pgorm;
use crate::entity::prelude::*;

/// The base of the disjoint [`Vehicle`]: one row per vehicle, its kind in
/// `kind`.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "vehicle")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub kind: VehicleKind,
    pub name: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}

/// A vehicle by its kind: a car and a truck each have a row of their own, a
/// bike none.
// [spec:pgorm:def:disjoint.declare+1]
#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = Entity)]
pub enum Vehicle {
    Car(super::car::Model),
    Truck(super::truck::Model),
    Bike,
}
