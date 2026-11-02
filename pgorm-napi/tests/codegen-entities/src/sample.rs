//! An entity of many column types, each of which the generated
//! declarations type as its records read it.

use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "samples", schema_name = "napi_entities")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i64,
    pub small: i16,
    pub ratio: f64,
    pub flag: bool,
    pub price: Decimal,
    pub token: Uuid,
    pub doc: Json,
    pub day: Date,
    pub at: DateTimeWithTimeZone,
    pub raw: Vec<u8>,
    pub tags: Vec<String>,
    pub note: Option<String>,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
