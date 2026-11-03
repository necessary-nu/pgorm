use crate as pgorm;
use crate::entity::prelude::*;

/// The kind `apartment` of [`Listing`](super::listing::Listing), keyed by
/// the listing's two columns under other names.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "flat", schema_name = "market")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub tenant: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub listing: i64,
    pub rooms: i16,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
