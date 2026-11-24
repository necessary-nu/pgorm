//! Scoped entities: a company's notes, each written by one of its users, and
//! messages a user sent or received.

use crate as pgorm;
use crate::entity::prelude::*;

/// A company, the scope of everything its members read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, ValueType, Scope)]
pub struct CompanyId(pub i64);

/// A user, the scope of what one member wrote or was sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, ValueType, Scope)]
pub struct UserId(pub i64);

/// A member of one company, signed in as one of its users.
#[derive(Clone, Copy, Debug, ScopeContext)]
pub struct Member {
    pub company: CompanyId,
    pub user: UserId,
}

/// An administrator of one company, over every one of its users.
#[derive(Clone, Copy, Debug, ScopeContext)]
pub struct CompanyAdmin {
    pub company: CompanyId,
    pub users: Every<UserId>,
}

pub mod company {
    use super::{CompanyId, UserId};
    use crate as pgorm;
    use crate::entity::prelude::*;

    /// A member reads their own company's row alone: its key is the scope.
    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "company")]
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
    use crate as pgorm;
    use crate::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "note")]
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

pub mod message {
    use super::{CompanyId, UserId};
    use crate as pgorm;
    use crate::entity::prelude::*;
    use crate::pgorm_query::Name;

    /// A message is its sender's and its recipient's to read, which no one
    /// column says: its `UserId` scope is written by hand.
    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "message", scope(UserId))]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i64,
        pub sender_id: UserId,
        pub recipient_id: UserId,
        pub body: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}

    impl ScopeOn<Entity> for UserId {
        fn condition(&self, table: Name) -> Condition {
            Condition::any()
                .add(Expr::col((table.clone(), Column::SenderId)).eq(self.0))
                .add(Expr::col((table, Column::RecipientId)).eq(self.0))
        }

        fn insert(&self, row: &mut ActiveModel) -> Result<(), ScopeError> {
            match &row.sender_id {
                ActiveValue::NotSet => {
                    row.sender_id = set(*self);
                    Ok(())
                }
                ActiveValue::Set(sender) | ActiveValue::Unchanged(sender) if sender == self => {
                    Ok(())
                }
                _ => Err(ScopeError::Refused(
                    "a message is sent by the user writing it".to_owned(),
                )),
            }
        }
    }
}
