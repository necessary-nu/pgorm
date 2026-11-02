use crate as pgorm;
use crate::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(schema_name = "public", table_name = "cake_filling_price")]
pub struct Model {
    #[pgorm(primary_key)]
    pub cake_id: i32,
    #[pgorm(primary_key)]
    pub filling_id: i32,
    pub price: Decimal,
    #[pgorm(ignore)]
    pub ignored_attr: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = "super::cake_filling::Entity",
        from = "(Column::CakeId, Column::FillingId)",
        to = "(super::cake_filling::Column::CakeId, super::cake_filling::Column::FillingId)"
    )]
    CakeFilling,
}

impl Related<super::cake_filling::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::CakeFilling.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
