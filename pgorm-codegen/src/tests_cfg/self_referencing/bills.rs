use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "bills")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub self_id: Option<i32> ,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = "Entity",
        from = "Column::SelfId",
        to = "Column::Id",
    )]
    SelfRef,
}

impl ActiveModelBehavior for ActiveModel {}
