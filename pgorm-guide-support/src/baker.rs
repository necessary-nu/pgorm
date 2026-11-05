use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "baker")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub name: String,
    pub hired_on: Date,
    pub bakery_id: i32,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::bakery::Entity,
        from = Column::BakeryId,
        to = super::bakery::Column::Id
    )]
    Bakery,
}

impl Related<super::cake::Entity> for Entity {
    fn to() -> RelationDef {
        super::cake_baker::Relation::Cake.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::cake_baker::Relation::Baker.def().rev())
    }
}
