use pgorm::entity::prelude::*;

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(
        has_many = super::Nothing,
        on_condition = |right| Expr::col((right, Column::Id)).gt(1)
    )]
    Nothing,
}

fn main() {}
