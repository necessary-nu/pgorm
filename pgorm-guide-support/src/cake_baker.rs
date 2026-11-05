use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "cake_baker")]
pub struct Model {
    #[pgorm(primary_key)]
    pub cake_id: i32,
    #[pgorm(primary_key)]
    pub baker_id: i32,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::cake::Entity,
        from = Column::CakeId,
        to = super::cake::Column::Id,
        on_delete = ForeignKeyAction::Cascade
    )]
    Cake,
    #[pgorm(
        belongs_to = super::baker::Entity,
        from = Column::BakerId,
        to = super::baker::Column::Id,
        on_delete = ForeignKeyAction::Cascade
    )]
    Baker,
}
