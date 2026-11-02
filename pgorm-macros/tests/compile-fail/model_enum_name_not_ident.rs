use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Model)]
pub struct Model {
    #[pgorm(enum_name = "1st")]
    pub id: i32,
}

fn main() {}
