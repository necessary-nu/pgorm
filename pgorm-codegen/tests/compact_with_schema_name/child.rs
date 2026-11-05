use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(schema_name = "schema_name", table_name = "child")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub parent_id1: i32,
    pub parent_id2: i32,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::parent::Entity,
        from = (Column::ParentId1, Column::ParentId2),
        to = (super::parent::Column::Id1, super::parent::Column::Id2),
    )]
    Parent,
}
