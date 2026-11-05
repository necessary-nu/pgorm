use pgorm::entity::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ActiveEnum)]
pub enum Flavor {
    Chocolate,
    Vanilla,
    Lemon,
    Carrot,
}

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "cake")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub name: String,
    pub flavor: Flavor,
    #[pgorm(column_type = ColumnType::Decimal(Some((10, 2))))]
    pub price: Decimal,
    pub gluten_free: bool,
    pub bakery_id: i32,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::bakery::Entity,
        from = Column::BakeryId,
        to = super::bakery::Column::Id
    )]
    Bakery,
    #[pgorm(has_many = super::order_line::Entity)]
    OrderLine,
}

impl Related<super::baker::Entity> for Entity {
    fn to() -> RelationDef {
        super::cake_baker::Relation::Baker.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::cake_baker::Relation::Cake.def().rev())
    }
}
