use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "metadata")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub uuid: Uuid,
    #[pgorm(column_name = "type", variant = Type)]
    pub ty: String,
    pub key: String,
    pub value: String,
    pub bytes: Vec<u8>,
    pub date: Option<Date>,
    pub time: Option<Time>,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
