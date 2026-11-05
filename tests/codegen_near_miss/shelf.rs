use pgorm::entity::prelude::*;

use super::pgorm_active_enums::ShelfKind;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(
    table_name = "shelf",
    unique(name = "uq-shelf-id-kind", columns = (Column::Id, Column::Kind)),
    unique(nulls_not_distinct, columns = (Column::Aisle, Column::Label))
)]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub kind: ShelfKind,
    pub aisle: Option<i32>,
    pub label: Option<String>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_one = super::book::Entity)]
    Book,
}
