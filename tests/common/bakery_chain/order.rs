use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "order")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::Decimal(Some((16, 4))))]
    pub total: Decimal,
    pub bakery_id: i32,
    pub customer_id: i32,
    pub placed_at: DateTime,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::bakery::Entity,
        from = Column::BakeryId,
        to = super::bakery::Column::Id
    )]
    Bakery,
    #[pgorm(
        belongs_to = super::customer::Entity,
        from = Column::CustomerId,
        to = super::customer::Column::Id,
        on_update = ForeignKeyAction::Cascade,
        on_delete = ForeignKeyAction::Cascade
    )]
    Customer,
    #[pgorm(has_many = super::lineitem::Entity)]
    Lineitem,
}
