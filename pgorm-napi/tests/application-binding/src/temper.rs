//! The temper of each mood an account can be in, keyed by the mood as the
//! domain `napi_entities.temper` over the account's `Mood` enum, and naming
//! the mood it settles into. PostgreSQL implements no foreign key between two
//! columns of a domain over an enum (`42804`), so schema generation refuses
//! the `Settles` relation; registered so that JavaScript meets the refusal.

use pgorm::entity::prelude::*;

use super::account::Mood;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[pgorm(table_name = "tempers", schema_name = "napi_entities")]
pub struct Model {
    #[pgorm(
        primary_key,
        auto_increment = false,
        column_type = ColumnType::Domain { name: Name::runtime("temper"), schema: Some(Name::runtime("napi_entities")), base: Arc::new(Mood::db_type().get_column_type().clone()) }
    )]
    pub mood: Mood,
    #[pgorm(
        column_type = ColumnType::Domain { name: Name::runtime("temper"), schema: Some(Name::runtime("napi_entities")), base: Arc::new(Mood::db_type().get_column_type().clone()) }
    )]
    pub settles_into: Mood,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[pgorm(
        belongs_to = "Entity",
        from = "Column::SettlesInto",
        to = "Column::Mood"
    )]
    Settles,
}

impl ActiveModelBehavior for ActiveModel {}
