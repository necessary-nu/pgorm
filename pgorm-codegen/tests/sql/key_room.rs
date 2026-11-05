use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "key_room")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::key_stay::Entity)]
    KeyStay,
}
