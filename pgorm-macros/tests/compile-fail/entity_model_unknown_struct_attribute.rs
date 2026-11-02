use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "cake", tabel_iden)]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
}

fn main() {}
