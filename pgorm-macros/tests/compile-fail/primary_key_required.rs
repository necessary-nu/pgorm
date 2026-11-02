use pgorm::entity::prelude::*;

// An entity is read and written by its key, so a `Model` with no
// `#[pgorm(primary_key)]` field is refused at the struct.
#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "posts")]
pub struct Model {
    pub title: String,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

fn main() {}
