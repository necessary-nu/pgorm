use crate as pgorm;
use crate::entity::prelude::*;

/// The kind `archived` of [`Exhibit`](super::gallery_exhibit::Exhibit), the
/// table `exhibit` in the schema `archive`.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "exhibit", schema_name = "archive")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub shelf: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::gallery_exhibit::Entity,
        from = Column::Id,
        to = super::gallery_exhibit::Column::Id
    )]
    Exhibit,
}
