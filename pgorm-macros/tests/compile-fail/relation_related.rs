mod cake {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "cake")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod related_true {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(has_many = super::cake::Entity, related = true)]
        Cake,
    }
}

mod reached_twice {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(has_many = super::cake::Entity, related = false)]
        Cake,
        #[pgorm(has_many = super::cake::Entity)]
        OtherCake,
    }
}

mod hooks_on_relation {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Debug, Relation)]
    #[pgorm(hooks)]
    pub enum Relation {}
}

fn main() {}
