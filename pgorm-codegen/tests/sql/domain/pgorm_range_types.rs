use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, CreatedRange)]
#[pgorm(range_name = "floatrange")]
pub struct Floatrange(pub Range<f64>);
