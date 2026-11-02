#![allow(unused_imports)]

use pgorm::entity::prelude::*;
use pgorm::tests_cfg::cake::Entity as Cake;
use pgorm::tests_cfg::{cake, filling};

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
#[pgorm(entity = Cake)]
pub enum Relation {
    #[pgorm(
        belongs_to = "filling::Entity",
        from = "cake::Column::Id",
        to = "filling::Column::Id",
        enforcement = "Lax"
    )]
    Filling,
}

fn main() {}
