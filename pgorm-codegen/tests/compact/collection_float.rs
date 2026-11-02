use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "collection_float")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub floats: Vec<f32> ,
    pub doubles: Vec<f64> ,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
