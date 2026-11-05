use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "vendor")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    #[pgorm(column_name = "_name_", column_type = ColumnType::String(StringLen::N(255)))]
    pub name: String,
    #[pgorm(column_name = "fruitId")]
    pub fruit_id: Option<i32>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::fruit::Entity,
        from = Column::FruitId,
        to = super::fruit::Column::Id
    )]
    Fruit,
}
