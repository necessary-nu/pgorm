use crate as pgorm;
use crate::entity::prelude::*;

/// The base of the disjoint [`Exhibit`], the table `exhibit` in the schema
/// `gallery`, whose two kinds' tables are each named `exhibit` too, in the
/// schemas `archive` and `loan`.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "exhibit", schema_name = "gallery")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub kind: ExhibitKind,
    pub title: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_one = super::archive_exhibit::Entity)]
    Archived,
    #[pgorm(has_one = super::loan_exhibit::Entity)]
    Loaned,
}

/// An exhibit by its kind: archived and loaned exhibits each have a row in a
/// table of their own, both named as the base's, and a hung one none.
// [spec:pgorm:def:disjoint.declare+5]
#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = Entity, schema_name = "gallery")]
pub enum Exhibit {
    Archived(super::archive_exhibit::Model),
    Loaned(super::loan_exhibit::Model),
    Hung,
}
