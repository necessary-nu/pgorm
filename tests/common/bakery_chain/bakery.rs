use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "bakery")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub name: String,
    pub profit_margin: f64,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::baker::Entity)]
    Baker,
    #[pgorm(has_many = super::order::Entity)]
    Order,
    #[pgorm(has_many = super::cake::Entity)]
    Cake,
}
