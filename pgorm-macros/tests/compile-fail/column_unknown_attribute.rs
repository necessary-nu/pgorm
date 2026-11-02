use pgorm::entity::prelude::*;

#[derive(Copy, Clone, Debug, EnumIter, Column)]
pub enum Column {
    #[pgorm(colum_name = "id")]
    Id,
}

fn main() {}
