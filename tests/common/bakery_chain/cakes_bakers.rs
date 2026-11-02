use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "cakes_bakers")]
pub struct Model {
    #[pgorm(primary_key)]
    pub cake_id: i32,
    #[pgorm(primary_key)]
    pub baker_id: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = "super::cake::Entity",
        from = "Column::CakeId",
        to = "super::cake::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Cake,
    #[pgorm(
        belongs_to = "super::baker::Entity",
        from = "Column::BakerId",
        to = "super::baker::Column::Id"
    )]
    Baker,
}

impl ActiveModelBehavior for ActiveModel {}
