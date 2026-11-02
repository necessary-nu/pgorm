//! An application's parcel: every column but its key typed with a domain the
//! schema creates — over an integer, over the account's enum, and an array of
//! one over text — so the binding is seen to read, write and compare through
//! them as through their base types.

use pgorm::entity::prelude::*;

use super::account::Mood;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "parcels", schema_name = "napi_entities")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    #[pgorm(
        column_type = ColumnType::Domain { name: Name::runtime("quantity"), schema: Some(Name::runtime("napi_entities")), base: Arc::new(ColumnType::Integer) }
    )]
    pub qty: i32,
    #[pgorm(
        column_type = ColumnType::Domain { name: Name::runtime("temper"), schema: Some(Name::runtime("napi_entities")), base: Arc::new(Mood::db_type().get_column_type().clone()) }
    )]
    pub mood: Mood,
    #[pgorm(
        column_type = ColumnType::Array(Arc::new(ColumnType::Domain { name: Name::runtime("code"), schema: Some(Name::runtime("napi_entities")), base: Arc::new(ColumnType::Text) }))
    )]
    pub codes: Vec<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
