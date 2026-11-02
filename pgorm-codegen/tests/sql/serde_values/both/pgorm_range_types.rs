use pgorm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, CreatedRange, Serialize, Deserialize)]
#[pgorm(range_name = "slot", schema_name = "booking")]
pub struct Slot(pub Range<i32>);
#[derive(Clone, Debug, PartialEq, Eq, CreatedRange, Serialize, Deserialize)]
#[pgorm(multirange_name = "slot_multirange", schema_name = "booking")]
pub struct SlotMultirange(pub Multirange<i32>);
#[derive(Clone, Debug, PartialEq, CreatedRange, Serialize, Deserialize)]
#[pgorm(multirange_name = "floatmultirange")]
pub struct Floatmultirange(pub Multirange<f64>);
#[derive(Clone, Debug, PartialEq, CreatedRange, Serialize, Deserialize)]
#[pgorm(range_name = "floatrange")]
pub struct Floatrange(pub Range<f64>);
