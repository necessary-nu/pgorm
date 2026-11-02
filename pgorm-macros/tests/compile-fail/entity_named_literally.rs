use pgorm::entity::prelude::*;

// A literal name is the composite derive's to write; the entity-trait derive
// stands beside a hand-written `EntityName`, for a name computed at run time.
#[derive(Copy, Clone, Default, Debug, DeriveEntity)]
#[pgorm(table_name = "fruit")]
pub struct Entity;

fn main() {}
