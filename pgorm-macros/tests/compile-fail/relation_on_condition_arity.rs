use pgorm::entity::prelude::*;

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[pgorm(
        has_many = "super::Nothing",
        on_condition = r#"|right| Expr::col((right, Column::Id)).gt(1)"#
    )]
    Nothing,
}

fn main() {}
