mod read_both_ways {
    use pgorm::entity::prelude::*;
    use pgorm::pgorm_query::SimpleExpr;

    fn as_text(expr: Expr) -> SimpleExpr {
        expr.cast_as(Name::runtime("text"))
    }

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "read_both_ways")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(select_as = "text", select_with = as_text)]
        pub name: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod written_without_arrays {
    use pgorm::entity::prelude::*;
    use pgorm::pgorm_query::SimpleExpr;

    fn as_text(val: Expr) -> SimpleExpr {
        val.cast_as(Name::runtime("text"))
    }

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "written_without_arrays")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(save_with = as_text)]
        pub name: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod arrays_without_values {
    use pgorm::entity::prelude::*;
    use pgorm::pgorm_query::SimpleExpr;

    fn as_texts(val: Expr) -> SimpleExpr {
        val.cast_as(Name::runtime("text"))
    }

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "arrays_without_values")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(save_as = "text", save_array_with = as_texts)]
        pub name: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod quoted_function {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "quoted_function")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(select_with = "as_text")]
        pub name: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

fn main() {}
