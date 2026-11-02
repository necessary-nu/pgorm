use pgorm::entity::prelude::*;

/// A table called `old`, the name PostgreSQL 18's RETURNING gives a written
/// row as it stood before the write: a write returning both has to rename
/// them, or the table would answer for the keyword.
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "old", schema_name = "python_entities")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub label: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
