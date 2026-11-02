mod both_kinds {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "both_kinds")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(generated_stored = Expr::col(Column::Id), generated_virtual = Expr::col(Column::Id))]
        pub copy: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod beside_identity {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "beside_identity")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(identity, generated_stored = Expr::col(Column::Id))]
        pub copy: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod beside_default {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "beside_default")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(generated_virtual = Expr::col(Column::Id), default_value = 1)]
        pub copy: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod beside_auto_increment {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "beside_auto_increment")]
    pub struct Model {
        #[pgorm(primary_key, generated_stored = Expr::col(Column::Base), auto_increment = false)]
        pub id: i32,
        pub base: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod virtual_key {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "virtual_key")]
    pub struct Model {
        #[pgorm(primary_key, generated_virtual = Expr::col(Column::Base))]
        pub id: i32,
        pub base: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod virtual_unique {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "virtual_unique")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(unique, generated_virtual = Expr::col(Column::Id))]
        pub copy: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod virtual_indexed {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "virtual_indexed")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(generated_virtual = Expr::col(Column::Id), indexed)]
        pub copy: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

fn main() {}
