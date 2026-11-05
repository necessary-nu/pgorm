use pgorm::entity::prelude::*;

use super::pgorm_active_enums::Grade;
use super::pgorm_range_types::Textrange;

#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "specimen")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::String(StringLen::N(3)))]
    pub code: String,
    #[pgorm(column_type = ColumnType::String(StringLen::N(255)), nullable)]
    pub title: Option<String>,
    pub note: String,
    #[pgorm(column_type = ColumnType::Char(Some(2)))]
    pub initials: String,
    #[pgorm(column_type = ColumnType::Char(Some(1)))]
    pub flag: String,
    #[pgorm(column_type = ColumnType::Char(None))]
    pub pad: String,
    #[pgorm(column_type = ColumnType::Text)]
    pub body: String,
    #[pgorm(
        column_type = ColumnType::Array(Arc::new(ColumnType::String(StringLen::N(3))))
    )]
    pub codes: Vec<String>,
    #[pgorm(
        column_type = ColumnType::Array(Arc::new(ColumnType::Char(Some(4)))),
        nullable
    )]
    pub marks: Option<Vec<String>>,
    pub notes: Vec<String>,
    #[pgorm(column_type = ColumnType::Decimal(Some((10, 2))))]
    pub price: Decimal,
    #[pgorm(column_type = ColumnType::Decimal(Some((6, 0))))]
    pub whole: Decimal,
    pub amount: Decimal,
    #[pgorm(column_type = ColumnType::Array(Arc::new(ColumnType::Decimal(Some((8, 3))))))]
    pub prices: Vec<Decimal>,
    #[pgorm(column_type = ColumnType::Money, nullable)]
    pub cost: Option<Decimal>,
    #[pgorm(column_type = ColumnType::Array(Arc::new(ColumnType::Money)))]
    pub fees: Vec<Decimal>,
    pub small: i16,
    pub big: i64,
    #[pgorm(column_type = ColumnType::Float)]
    pub ratio: f32,
    #[pgorm(column_type = ColumnType::Double)]
    pub weight: f64,
    pub doc: Json,
    #[pgorm(column_type = ColumnType::JsonBinary)]
    pub data: Json,
    #[pgorm(column_type = ColumnType::Bytea)]
    pub blob: Vec<u8>,
    pub ok: bool,
    pub uid: Uuid,
    pub day: Date,
    pub clock: Time,
    pub seen: DateTime,
    pub stamped: DateTimeWithTimeZone,
    pub wait: Interval,
    #[pgorm(
        column_type = ColumnType::Interval(IntervalSpec::Fields(PgInterval::YearToMonth))
    )]
    pub months: Interval,
    pub span: Range<i32>,
    pub days: Multirange<Date>,
    pub level: Grade,
    #[pgorm(
        column_type = ColumnType::CreatedRange {
            name: Name::runtime("textrange"),
            schema: None,
            subtype: Arc::new(ColumnType::Text),
        }
    )]
    pub letters: Textrange,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
