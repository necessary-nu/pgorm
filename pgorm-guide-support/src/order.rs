use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "order")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub customer_id: i32,
    pub bakery_id: i32,
    pub placed_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::customer::Entity,
        from = Column::CustomerId,
        to = super::customer::Column::Id
    )]
    Customer,
    #[pgorm(
        belongs_to = super::bakery::Entity,
        from = Column::BakeryId,
        to = super::bakery::Column::Id
    )]
    Bakery,
    #[pgorm(has_many = super::order_line::Entity)]
    OrderLine,
}
