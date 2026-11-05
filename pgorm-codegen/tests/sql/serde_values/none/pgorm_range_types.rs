use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
#[pgorm(range_name = "slot", schema_name = "booking")]
pub struct Slot(pub Range<i32>);

#[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
#[pgorm(multirange_name = "slot_multirange", schema_name = "booking")]
pub struct SlotMultirange(pub Multirange<i32>);

#[derive(Clone, Debug, PartialEq, CreatedRange)]
#[pgorm(multirange_name = "floatmultirange")]
pub struct Floatmultirange(pub Multirange<f64>);

#[derive(Clone, Debug, PartialEq, CreatedRange)]
#[pgorm(range_name = "floatrange")]
pub struct Floatrange(pub Range<f64>);
