use pgorm::entity::prelude::*;

// Case conversion drops non-alphanumerics, so `__` derives the empty string,
// which spells no `Column` variant.
#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "cake")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub __: i32,
}

fn main() {}
