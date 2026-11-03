use crate as pgorm;
use crate::entity::prelude::*;

/// The kind `loaned` of [`Exhibit`](super::gallery_exhibit::Exhibit), the
/// table `exhibit` in the schema `loan`.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "exhibit", schema_name = "loan")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub lender: String,
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
