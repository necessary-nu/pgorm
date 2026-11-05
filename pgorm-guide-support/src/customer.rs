use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "customer")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub name: String,
    #[pgorm(unique)]
    pub email: String,
    pub notes: Option<String>,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::order::Entity)]
    Order,
}
