mod both_forms {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "both_forms")]
    pub struct Model {
        #[pgorm(primary_key, identity, identity_by_default)]
        pub id: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod beside_auto_increment {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "beside_auto_increment")]
    pub struct Model {
        #[pgorm(primary_key, identity, auto_increment = false)]
        pub id: i32,
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
        #[pgorm(identity_by_default, default_value = 1)]
        pub seq: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod nullable {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "nullable")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(identity)]
        pub seq: Option<i32>,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod composite_serial {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "composite_serial")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub tenant_id: i32,
        #[pgorm(primary_key, auto_increment = true)]
        pub id: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

fn main() {}
