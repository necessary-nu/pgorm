mod thirteen_columns {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "thirteen_columns")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub k1: i32,
        #[pgorm(primary_key)]
        pub k2: i32,
        #[pgorm(primary_key)]
        pub k3: i32,
        #[pgorm(primary_key)]
        pub k4: i32,
        #[pgorm(primary_key)]
        pub k5: i32,
        #[pgorm(primary_key)]
        pub k6: i32,
        #[pgorm(primary_key)]
        pub k7: i32,
        #[pgorm(primary_key)]
        pub k8: i32,
        #[pgorm(primary_key)]
        pub k9: i32,
        #[pgorm(primary_key)]
        pub k10: i32,
        #[pgorm(primary_key)]
        pub k11: i32,
        #[pgorm(primary_key)]
        pub k12: i32,
        #[pgorm(primary_key)]
        pub k13: i32,
        pub name: String,
    }
}

mod key_given_twice {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "key_given_twice")]
    pub struct Model {
        #[pgorm(primary_key, primary_key)]
        pub id: i32,
    }
}

fn main() {}
