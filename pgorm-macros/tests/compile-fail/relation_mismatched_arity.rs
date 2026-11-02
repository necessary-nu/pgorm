#![allow(unused_imports)]

use pgorm::entity::prelude::*;
use pgorm::tests_cfg::cake::Entity as Cake;
use pgorm::tests_cfg::{cake, filling};

#[derive(Copy, Clone, Debug, Relation)]
#[pgorm(entity = Cake)]
pub enum Relation {
    #[pgorm(
        belongs_to = filling::Entity,
        from = (cake::Column::Id, cake::Column::Name),
        to = filling::Column::Id
    )]
    Filling,
}

fn main() {}
