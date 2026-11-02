use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Model)]
pub struct Model {
    #[pgorm(primary_ke)]
    pub id: i32,
}

fn main() {}
