pub mod value_type_general {
    use super::*;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "value_type")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub number: Integer,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod value_type_pg {
    use super::*;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "value_type_postgres")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub number: Integer,
        pub str_vec: StringVec,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, ValueType)]
#[pgorm(array_type = ArrayType::Int)]
pub struct Integer(i32);

impl<T> From<T> for Integer
where
    T: Into<i32>,
{
    fn from(v: T) -> Integer {
        Integer(v.into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, ValueType)]
#[pgorm(column_type = ColumnType::Boolean, array_type = ArrayType::Bool)]
pub struct Boolbean(pub String);

#[derive(Clone, Debug, PartialEq, Eq, ValueType)]
pub struct StringVec(pub Vec<String>);
