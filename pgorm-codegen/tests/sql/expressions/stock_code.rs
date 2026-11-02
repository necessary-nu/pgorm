use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "stock_code")]
pub struct Model {
    pub base: i32,
    #[pgorm(
        primary_key,
        generated_stored = "Expr::col(Column::Base).mul(Expr::val(2))"
    )]
    pub code: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
