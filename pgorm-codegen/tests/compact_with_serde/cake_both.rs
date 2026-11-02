use pgorm::entity::prelude:: * ;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Entity, Eq, Serialize, Deserialize)]
#[pgorm(table_name = "cake")]
pub struct Model {
    #[pgorm(primary_key)]
    #[serde(skip_deserializing)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::Text, nullable)]
    pub name: Option<String> ,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(has_many = "super::fruit::Entity")]
    Fruit,
}

impl Related<super::fruit::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Fruit.def()
    }
}

impl Related<super::filling::Entity> for Entity {
    fn to() -> RelationDef {
        super::cake_filling::Relation::Filling.def()
    }
    fn via() -> Option<RelationDef> {
        Some(super::cake_filling::Relation::Cake.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
