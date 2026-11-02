use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "customer")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub name: String,
    #[pgorm(column_type = ColumnType::Text, nullable)]
    pub notes: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::order::Entity)]
    Order,
}

impl Related<super::order::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Order.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
