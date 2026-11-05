use pgorm::entity::prelude::*;

use super::pgorm_range_types::{Floatmultirange, Floatrange, Slot, SlotMultirange, Textrange};

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
        column_type = ColumnType::CreatedRange {
            name: Name::runtime("textrange"),
            schema: None,
            subtype: Arc::new(ColumnType::Text),
        }
    )]
    pub label: Textrange,
    pub spares: Option<Vec<Floatrange>>,
    pub ledgers: Option<Vec<Floatmultirange>>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
