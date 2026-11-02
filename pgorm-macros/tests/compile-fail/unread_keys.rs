#![allow(dead_code, unused_imports)]

use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Model)]
#[pgorm(table_name = "cake")]
pub struct ModelContainer {
    pub id: i32,
}

#[derive(Clone, Debug, PartialEq, Model)]
pub struct ModelField {
    #[pgorm(column_type = ColumnType::Text)]
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Model)]
pub struct ModelEnumName {
    #[pgorm(enum_name = "Name")]
    pub name: String,
}

mod composite_enum_name {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "cake")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(enum_name = "Title")]
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod relation_container {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Debug, Relation)]
    #[pgorm(table_name = "cake")]
    pub enum Relation {}
}

mod relation_twice {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(has_one = super::Child, has_many = super::Child)]
        Child,
    }
}

#[derive(Copy, Clone, Debug, Column)]
#[pgorm(table_name = "cake")]
pub enum ColumnContainer {
    Id,
}

#[derive(IntoActiveModel)]
pub struct FormField {
    #[pgorm(ignore)]
    pub name: String,
}

#[derive(FromQueryResult)]
pub struct DecodedProjection {
    #[pgorm(from_col = Column::Name)]
    pub name: String,
}

fn main() {}
