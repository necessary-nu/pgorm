use pgorm::entity::prelude::*;

use super::pgorm_active_enums::Mood;
use super::pgorm_range_types::Floatrange;

#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "stock_line")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    #[pgorm(
        column_type = ColumnType::Domain { name: Name::runtime("quantity"), schema: None, base: Arc::new(ColumnType::Integer) },
        nullable
    )]
    pub qty: Option<i32>,
    #[pgorm(
        column_type = ColumnType::Domain { name: Name::runtime("small_quantity"), schema: None, base: Arc::new(ColumnType::Domain { name: Name::runtime("quantity"), schema: None, base: Arc::new(ColumnType::Integer) }) }
    )]
    pub small: i32,
    #[pgorm(
        column_type = ColumnType::Domain { name: Name::runtime("code"), schema: None, base: Arc::new(ColumnType::String(StringLen::N(8u32))) }
    )]
    pub code: String,
    #[pgorm(
        column_type = ColumnType::Array(Arc::new(ColumnType::Domain { name: Name::runtime("code"), schema: None, base: Arc::new(ColumnType::String(StringLen::N(8u32))) }))
    )]
    pub codes: Vec<String>,
    #[pgorm(
        column_type = ColumnType::Domain { name: Name::runtime("cheerful"), schema: None, base: Arc::new(Mood::db_type().get_column_type().clone()) }
    )]
    pub mood: Mood,
    #[pgorm(
        column_type = ColumnType::Array(Arc::new(ColumnType::Domain { name: Name::runtime("cheerful"), schema: None, base: Arc::new(Mood::db_type().get_column_type().clone()) })),
        nullable
    )]
    pub moods: Option<Vec<Mood>>,
    #[pgorm(
        column_type = ColumnType::Domain { name: Name::runtime("labels"), schema: None, base: Arc::new(ColumnType::Array(Arc::new(ColumnType::Text))) },
        nullable
    )]
    pub labels: Option<Vec<String>>,
    #[pgorm(
        column_type = ColumnType::Domain { name: Name::runtime("span"), schema: None, base: Arc::new(ColumnType::CreatedRange { name: Name::runtime("floatrange"), schema: None, subtype: Arc::new(ColumnType::Double) }) },
        nullable
    )]
    pub span: Option<Floatrange>,
    #[pgorm(
        column_type = ColumnType::Domain { name: Name::runtime("amount"), schema: Some(Name::runtime("billing")), base: Arc::new(ColumnType::Decimal(Some((12u32, 2u32)))) }
    )]
    pub amount: Decimal,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
