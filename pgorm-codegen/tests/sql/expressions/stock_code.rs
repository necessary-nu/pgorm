use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "stock_code")]
pub struct Model {
    pub base: i32,
    #[pgorm(
        primary_key,
        generated_stored = Expr::col(Column::Base).mul(Expr::val(2))
    )]
    pub code: i32,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
