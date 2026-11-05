//! A booking over a range type the schema creates, so that JavaScript
//! compares a column of a type only the schema names, and an array of it.

use pgorm::entity::prelude::*;

/// A range type the application's schema creates over `float8`.
#[derive(Clone, Debug, PartialEq, CreatedRange)]
#[pgorm(range_name = "floatrange", schema_name = "napi_entities")]
pub struct FloatRange(pub Range<f64>);

#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "bookings", schema_name = "napi_entities")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub span: FloatRange,
    pub spares: Option<Vec<FloatRange>>,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
