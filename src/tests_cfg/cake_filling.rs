use crate as pgorm;
use crate::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "cake_filling")]
pub struct Model {
    #[pgorm(primary_key)]
    pub cake_id: i32,
    #[pgorm(primary_key)]
    pub filling_id: i32,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::cake::Entity,
        from = Column::CakeId,
        to = super::cake::Column::Id
    )]
    Cake,
    #[pgorm(
        belongs_to = super::filling::Entity,
        from = Column::FillingId,
        to = super::filling::Column::Id
    )]
    Filling,
}

impl Related<super::cake_filling_price::Entity> for Entity {
    fn to() -> RelationDef {
        super::cake_filling_price::Relation::CakeFilling.def().rev()
    }
}
