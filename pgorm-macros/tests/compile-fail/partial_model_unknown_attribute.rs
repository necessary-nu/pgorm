use pgorm::entity::prelude::*;

#[derive(PartialModel)]
#[pgorm(entity = "Entity")]
struct Partial {
    #[pgorm(from_colum = "name")]
    label: String,
}

fn main() {}
