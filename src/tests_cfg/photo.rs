use crate as pgorm;
use crate::entity::prelude::*;

/// A photo, one of the targets a [`Commentable`](super::comment::Commentable)
/// comment is on, keyed by a `bigint`.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "photo")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i64,
    pub url: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
