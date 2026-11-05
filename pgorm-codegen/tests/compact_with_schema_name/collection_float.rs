use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(schema_name = "schema_name", table_name = "collection_float")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub floats: Vec<f32> ,
    pub doubles: Vec<f64> ,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
