use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "cake_filling_price")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub cake_id: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub filling_id: i32,
    pub price: Decimal,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = "super::cake_filling::Entity",
        from = "(Column::CakeId, Column::FillingId)",
        to = "(super::cake_filling::Column::CakeId, super::cake_filling::Column::FillingId)",
    )]
    CakeFilling,
}

impl Related<super::cake_filling::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::CakeFilling.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}