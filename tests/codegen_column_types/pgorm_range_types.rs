use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveCreatedRange)]
#[pgorm(range_name = "textrange")]
pub struct Textrange(pub Range<String>);
