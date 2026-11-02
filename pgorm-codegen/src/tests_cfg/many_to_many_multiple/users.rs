use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[pgorm(table_name = "users")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::Text)]
    pub email: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[pgorm(has_many = "super::bills::Entity")]
    Bills,
}

impl Related<super::bills::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Bills.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
