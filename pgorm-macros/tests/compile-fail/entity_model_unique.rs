mod one_column {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "one_column", unique(columns = (Column::A)))]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub a: i32,
        pub b: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod column_twice {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "column_twice", unique(columns = (Column::A, Column::B, Column::A)))]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub a: i32,
        pub b: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod quoted_column {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "quoted_column", unique(columns = (Column::A, "b")))]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub a: i32,
        pub b: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod no_columns {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "no_columns", unique(name = "uq-a"))]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub a: i32,
        pub b: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod name_twice {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "name_twice", unique(name = "uq-a", name = "uq-b", columns = (Column::A, Column::B)))]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub a: i32,
        pub b: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod name_too_long {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "name_too_long", unique(name = "uq-name_too_long-a-b-and-a-good-many-more-words-than-fit-in-a-name", columns = (Column::A, Column::B)))]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub a: i32,
        pub b: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod unknown_key {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "unknown_key", unique(deferrable, columns = (Column::A, Column::B)))]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub a: i32,
        pub b: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod computed_names {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(unique(columns = (Column::A, Column::B)))]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub a: i32,
        pub b: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod not_a_column {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "not_a_column", unique(columns = (Column::A, Column::Nope)))]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub a: i32,
        pub b: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod virtual_column {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "virtual_column", unique(columns = (Column::A, Column::Doubled)))]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub a: i32,
        #[pgorm(generated_virtual = Expr::col(Column::A).mul(2))]
        pub doubled: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

fn main() {}
