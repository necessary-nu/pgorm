mod twice {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "twice")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(immutable, immutable)]
        pub amount: i64,
    }
}

mod ignored {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "ignored")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(ignore, immutable)]
        pub amount: i64,
    }
}

mod generated {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "generated")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(generated_stored = Expr::col(Column::Id), immutable)]
        pub copy: i32,
    }
}

fn main() {}
