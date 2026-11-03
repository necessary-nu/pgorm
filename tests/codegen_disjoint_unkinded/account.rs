use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(schema_name = "crm", table_name = "account")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub tenant: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i64,
    pub name: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_one = super::company::Entity)]
    Company,
    #[pgorm(has_one = super::person::Entity)]
    Person,
}

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = Entity, unkinded)]
pub enum Account {
    Person(super::person::Model),
    Company(super::company::Model),
}
