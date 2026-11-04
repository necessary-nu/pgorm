use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "remark")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub body: String,
    pub topic_id: Option<i32>,
    pub reply_to_id: Option<i32>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(belongs_to = Entity, from = Column::ReplyToId, to = Column::Id)]
    SelfRef,
    #[pgorm(
        belongs_to = super::topic::Entity,
        from = Column::TopicId,
        to = super::topic::Column::Id
    )]
    Topic,
}

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, OneOf)]
#[pgorm(entity = Entity)]
pub enum Subject {
    #[pgorm(column = Column::TopicId)]
    Topic(super::topic::Entity),
    #[pgorm(column = Column::ReplyToId)]
    Remark(Entity),
}
