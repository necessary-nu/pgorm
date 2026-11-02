use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "_cake_filling_")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub cake_id: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub filling_id: i32,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::cake::Entity,
        from = Column::CakeId,
        to = super::cake::Column::Id,
        on_update = ForeignKeyAction::Cascade,
        on_delete = ForeignKeyAction::Cascade,
    )]
    Cake,
    #[pgorm(
        belongs_to = super::filling::Entity,
        from = Column::FillingId,
        to = super::filling::Column::Id,
        on_update = ForeignKeyAction::Cascade,
        on_delete = ForeignKeyAction::Cascade,
    )]
    Filling,
}
