use pgorm::entity::prelude::*;

use super::pgorm_range_types::Floatmultirange;
use super::pgorm_range_types::Floatrange;
use super::pgorm_range_types::Slot;
use super::pgorm_range_types::SlotMultirange;
use super::pgorm_range_types::Textrange;

#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "measurement")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub spans: Floatmultirange,
    pub span: Floatrange,
    pub slot: Option<Slot>,
    pub slots: Option<SlotMultirange>,
    #[pgorm(
        column_type = ColumnType::CreatedRange { name: Name::runtime("textrange"), schema: None, subtype: Arc::new(ColumnType::Text) }
    )]
    pub label: Textrange,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
