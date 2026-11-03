//! A till whose takings are `money` and whose floats are an array of it:
//! columns an entity reads cast to `numeric` and writes cast to `money`, so
//! that JavaScript binds and reads them as decimals.

use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "tills", schema_name = "napi_entities")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::Money)]
    pub takings: Decimal,
    #[pgorm(column_type = ColumnType::Array(Arc::new(ColumnType::Money)))]
    pub floats: Option<Vec<Decimal>>,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
