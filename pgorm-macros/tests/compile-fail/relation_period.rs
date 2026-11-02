#![allow(unused_imports)]

mod one_side {
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
            from_period = "cake::Column::Name"
        )]
        Filling,
    }
}

mod with_an_action {
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
            from_period = "cake::Column::Name",
            to_period = "filling::Column::Name",
            on_delete = "Cascade"
        )]
        Filling,
    }
}

fn main() {}
