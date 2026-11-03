use crate as pgorm;
use crate::entity::prelude::*;

/// The base of the unkinded [`Shop`]: one row per shop, with no kind column,
/// a shop's kind being the table that has a row for it.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "shop")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub name: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}

/// A shop by which table has a row for it: a web shop and a storefront each
/// have one, a dormant shop neither.
// [spec:pgorm:sem:disjoint.unkinded]
#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = Entity, unkinded)]
pub enum Shop {
    WebShop(super::web_shop::Model),
    Storefront(super::storefront::Model),
    Dormant,
}
