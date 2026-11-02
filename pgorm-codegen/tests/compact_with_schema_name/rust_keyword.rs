use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(schema_name = "schema_name", table_name = "rust_keyword")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub testing: i16,
    pub rust: i16,
    pub keywords: i16,
    pub r#type: i16,
    pub r#typeof: i32,
    pub crate_: i64,
    pub self_: i64,
    pub self_id1: i64,
    pub self_id2: i32,
    pub fruit_id1: i32,
    pub fruit_id2: i32,
    pub cake_id: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = "Entity",
        from = "Column::SelfId1",
        to = "Column::Id",
    )]
    SelfRef1,
    #[pgorm(
        belongs_to = "Entity",
        from = "Column::SelfId2",
        to = "Column::Id",
    )]
    SelfRef2,
    #[pgorm(
        belongs_to = "super::fruit::Entity",
        from = "Column::FruitId1",
        to = "super::fruit::Column::Id",
    )]
    Fruit1,
    #[pgorm(
        belongs_to = "super::fruit::Entity",
        from = "Column::FruitId2",
        to = "super::fruit::Column::Id",
    )]
    Fruit2,
    #[pgorm(
        belongs_to = "super::cake::Entity",
        from = "Column::CakeId",
        to = "super::cake::Column::Id",
    )]
    Cake,
}

impl Related<super::cake::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Cake.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
