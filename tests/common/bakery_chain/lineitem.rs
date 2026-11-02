use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[pgorm(table_name = "lineitem")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::Decimal(Some((16, 4))))]
    pub price: Decimal,
    pub quantity: i32,
    pub order_id: i32,
    pub cake_id: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[pgorm(
        belongs_to = "super::order::Entity",
        from = "Column::OrderId",
        to = "super::order::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Order,
    #[pgorm(
        belongs_to = "super::cake::Entity",
        from = "Column::CakeId",
        to = "super::cake::Column::Id"
    )]
    Cake,
}

impl Related<super::order::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Order.def()
    }
}

impl Related<super::cake::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Cake.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
