use crate as pgorm;
use crate::entity::prelude::*;

/// A comment on a post or a photo, or on neither: one nullable column per
/// target, each its own relation's foreign key.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "comment")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub body: String,
    pub post_id: Option<i32>,
    pub photo_id: Option<i64>,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::photo::Entity,
        from = Column::PhotoId,
        to = super::photo::Column::Id
    )]
    Photo,
    #[pgorm(
        belongs_to = super::post::Entity,
        from = Column::PostId,
        to = super::post::Column::Id
    )]
    Post,
}

/// What a comment is on: a post, a photo, or nothing.
// [spec:pgorm:def:disjoint.one-of+3]
#[derive(Clone, Copy, Debug, OneOf)]
#[pgorm(entity = Entity, model_attrs(derive(PartialEq, Eq)))]
pub enum Commentable {
    #[pgorm(column = Column::PostId)]
    Post(super::post::Entity),
    #[pgorm(column = Column::PhotoId)]
    Photo(super::photo::Entity),
    Detached,
}
