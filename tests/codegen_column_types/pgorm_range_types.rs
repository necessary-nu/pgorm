use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
#[pgorm(range_name = "textrange")]
pub struct Textrange(pub Range<String>);
