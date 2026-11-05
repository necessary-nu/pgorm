use pgorm::entity::prelude::*;

use super::pgorm_active_enums::ShelfKind;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "book")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub title: String,
    #[pgorm(generated_stored = Expr::val("book"))]
    pub kind: Option<ShelfKind>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::shelf::Entity,
        from = (Column::Id, Column::Kind),
        to = (super::shelf::Column::Id, super::shelf::Column::Kind),
        on_delete = ForeignKeyAction::Cascade,
        deferrability = Deferrability::DeferrableInitiallyImmediate,
    )]
    Shelf,
}
