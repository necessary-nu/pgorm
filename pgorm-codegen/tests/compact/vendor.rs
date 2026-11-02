use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "vendor")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    #[pgorm(column_name = "_name_", column_type = ColumnType::String(StringLen::N(255u32)))]
    pub name: String,
    #[pgorm(column_name = "fruitId")]
    pub fruit_id: Option<i32> ,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::fruit::Entity,
        from = Column::FruitId,
        to = super::fruit::Column::Id,
    )]
    Fruit,
}

impl Related<super::fruit::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Fruit.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
