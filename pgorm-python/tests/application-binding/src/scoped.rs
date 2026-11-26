//! A company's notes, each held to its company and to the user who wrote
//! it: scoped entities, registered with their scopes so every statement
//! Python builds on them carries the scopes, as Rust's `_for` forms do.

use pgorm::entity::prelude::*;

/// A company, the scope its members' notes are held to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueType, Scope)]
pub struct CompanyId(pub i64);

/// A user, the scope what one member wrote is held to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueType, Scope)]
pub struct UserId(pub i64);

pub mod company {
    use super::CompanyId;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(schema_name = "python_scopes", table_name = "company")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false, scope)]
        pub id: CompanyId,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(has_many = super::note::Entity)]
        Note,
    }
}

pub mod note {
    use super::{CompanyId, UserId};
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(schema_name = "python_scopes", table_name = "note")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i64,
        #[pgorm(scope)]
        pub company_id: CompanyId,
        #[pgorm(scope)]
        pub author_id: UserId,
        pub body: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::company::Entity,
            from = Column::CompanyId,
            to = super::company::Column::Id
        )]
        Company,
    }
}
