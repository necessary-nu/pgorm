use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(schema_name = "schema_name", table_name = "child")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub parent_id1: i32,
    pub parent_id2: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::parent::Entity,
        from = (Column::ParentId1, Column::ParentId2),
        to = (super::parent::Column::Id1, super::parent::Column::Id2),
    )]
    Parent,
}

impl Related<super::parent::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Parent.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
