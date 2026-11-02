use pgorm::entity::prelude::*;

// `_1` derives `"1"`, which is a literal rather than an identifier.
#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "cake")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub _1: i32,
}

fn main() {}
