use pgorm::entity::prelude::*;

// The `Entity` derive's input MUST be a struct named exactly `Model`.
#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "cake")]
pub struct Cake {
    #[pgorm(primary_key)]
    pub id: i32,
}

fn main() {}
