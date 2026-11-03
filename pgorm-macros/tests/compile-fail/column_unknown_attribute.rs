use pgorm::entity::prelude::*;

#[derive(Copy, Clone, Debug, Column)]
pub enum Column {
    #[pgorm(colum_name = "id")]
    Id,
}

fn main() {}
