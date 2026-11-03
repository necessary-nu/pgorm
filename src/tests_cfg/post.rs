use crate as pgorm;
use crate::entity::prelude::*;

/// A post, one of the targets a [`Commentable`](super::comment::Commentable)
/// comment is on.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "post")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub title: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
