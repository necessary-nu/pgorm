use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "binary")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub binary: Vec<u8>,
    pub binary_10: Vec<u8>,
    pub var_binary_16: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
