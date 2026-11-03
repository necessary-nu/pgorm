use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "web_store")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub store_id: i32,
    pub url: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(belongs_to = super::store::Entity, from = Column::StoreId, to = super::store::Column::Id)]
    Store,
}
