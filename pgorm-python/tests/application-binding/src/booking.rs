use pgorm::entity::prelude::*;

/// A range type the application's schema creates over `float8`.
#[derive(Clone, Debug, PartialEq, CreatedRange)]
#[pgorm(range_name = "floatrange", schema_name = "python_entities")]
pub struct FloatRange(pub Range<f64>);

/// The multirange PostgreSQL creates beside it.
#[derive(Clone, Debug, PartialEq, CreatedRange)]
#[pgorm(multirange_name = "floatmultirange", schema_name = "python_entities")]
pub struct FloatSpans(pub Multirange<f64>);

/// A booking over a created range and its multirange, and an array of each,
/// registered so that Python reads and writes a type only the schema names.
#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "bookings", schema_name = "python_entities")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub span: FloatRange,
    pub spans: Option<FloatSpans>,
    pub spares: Option<Vec<FloatRange>>,
    pub ledgers: Option<Vec<FloatSpans>>,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
