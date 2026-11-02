use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "cake")]
pub struct Model {
    #[pgorm(primary_key, auto_incremnt = false)]
    pub id: i32,
}

fn main() {}
