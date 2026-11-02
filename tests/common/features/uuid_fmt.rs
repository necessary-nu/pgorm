use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "uuid_fmt")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub uuid: Uuid,
    pub uuid_braced: uuid::fmt::Braced,
    pub uuid_hyphenated: uuid::fmt::Hyphenated,
    pub uuid_simple: uuid::fmt::Simple,
    pub uuid_urn: uuid::fmt::Urn,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
