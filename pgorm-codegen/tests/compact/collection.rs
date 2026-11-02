use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "collection")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub integers: Vec<i32> ,
    pub integers_opt: Option<Vec<i32> > ,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
