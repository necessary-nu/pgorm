//! The scopes chapter's domain, beside the bakery: companies, the notes their
//! users write, and the messages users send each other. Every read and write
//! of a note is held to one company, and of a message to one user.
//!
//! | table | rows |
//! | --- | --- |
//! | `company` | 1 Acme, 2 Globex |
//! | `note` | 1 Ada's and 2 Bo's at Acme (users 10 and 11), 3 Cy's at Globex (user 20) |
//! | `message` | 1 Ada to Bo, 2 Bo to Ada, 3 Bo to Cy |

use pgorm::entity::prelude::*;
use pgorm::{ConnectionTrait, Schema};

/// A company, the scope everything its members read is held to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, ValueType, Scope)]
pub struct CompanyId(pub i64);

/// A user, the scope what one member wrote or was sent is held to.
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
    use super::CompanyId;
    use pgorm::entity::prelude::*;

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
    use pgorm::entity::prelude::*;

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
    use super::UserId;
    use pgorm::entity::prelude::*;

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

    /// A message is its sender's and its recipient's to read, and is sent by
    /// the user writing it.
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

/// Create the notes' tables, each from its entity.
pub async fn create_schema<C: ConnectionTrait>(db: &C) -> Result<(), Error> {
    let schema = Schema::new();
    for create in [
        schema.create_table_from_entity(company::Entity)?,
        schema.create_table_from_entity(note::Entity)?,
        schema.create_table_from_entity(message::Entity)?,
    ] {
        db.batch_execute(&create.to_string()).await?;
    }
    Ok(())
}

/// Insert the notes' rows, each under the context that may write it.
pub async fn seed<C: ConnectionTrait>(db: &C) -> Result<(), Error> {
    for (id, name) in [(1, "Acme"), (2, "Globex")] {
        Insert::one_for(
            &CompanyId(id),
            company::ActiveModel {
                name: set(name),
                ..Default::default()
            },
        )
        .exec(db)
        .await?;
    }
    for (company, user, body) in [
        (1, 10, "Ada's plan"),
        (1, 11, "Bo's draft"),
        (2, 20, "Cy's notes"),
    ] {
        let member = Member {
            company: CompanyId(company),
            user: UserId(user),
        };
        let note = note::ActiveModel {
            body: set(body),
            ..Default::default()
        };
        Insert::one_for(&member, note).exec(db).await?;
    }
    for (sender, recipient, body) in [(10, 11, "lunch?"), (11, 10, "yes"), (11, 20, "hello")] {
        let message = message::ActiveModel {
            recipient_id: set(UserId(recipient)),
            body: set(body),
            ..Default::default()
        };
        Insert::one_for(&UserId(sender), message).exec(db).await?;
    }
    Ok(())
}
