use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[pgorm(table_name = "posts")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    #[pgorm(column_type = "Text")]
    pub text: String,
}

impl ActiveModelBehavior for ActiveModel {}

fn main() {}
