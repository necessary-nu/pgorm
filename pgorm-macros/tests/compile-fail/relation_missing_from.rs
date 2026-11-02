use pgorm::entity::prelude::*;

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(belongs_to = super::Nothing)]
    Nothing,
}

fn main() {}
