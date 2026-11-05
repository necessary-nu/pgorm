use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "users")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub self_id: Option<i32>,
    pub self_idd: Option<i32>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(belongs_to = Entity, from = Column::SelfId, to = Column::Id)]
    SelfRef2,
    #[pgorm(belongs_to = Entity, from = Column::SelfIdd, to = Column::Id)]
    SelfRef1,
}
