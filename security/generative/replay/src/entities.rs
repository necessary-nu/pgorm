//! The campaign's entity, model and graph declarations.
//!
//! A copy of `security/generative/bridge/src/{account,note,room,stay,graphs}.rs`, which
//! is already PyO3-free — only the module paths differ. The hostile enum name
//! and the `before_save` hook are part of what the campaign tests, so they are
//! reproduced exactly rather than tidied: a replay whose entities are milder
//! than the binding's is replaying a different program.

use pgorm::{EntityTrait, Iterable, StaticName};

use crate::FormatError;

/// The compiled column a portable program named by its SQL identifier.
///
/// A generated program addresses columns by name, because that is all a
/// portable artifact can carry; the binding resolves the same name the same
/// way, so a name outside the compiled set is refused rather than guessed at.
///
/// # Errors
///
/// Returns [`FormatError`] when the entity declares no column with this name.
pub fn column<E: EntityTrait>(name: &str) -> Result<E::Column, FormatError> {
    E::Column::iter()
        .find(|column| StaticName::as_str(column) == name)
        .ok_or_else(|| FormatError::new("unknown compiled entity column"))
}

/// The accounts table: every scalar kind the campaign exercises, a
/// schema-qualified enum whose name carries a quote and a non-ASCII character,
/// and a save hook that rewrites on insert and on update alike.
pub mod account {
    use pgorm::entity::prelude::*;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, EnumIter, ActiveEnum, Serialize, Deserialize)]
    #[pgorm(
        rs_type = String,
        enum_name = "State\" 雪",
        schema_name = "fixture"
    )]
    pub enum State {
        #[pgorm(string_value = "calm")]
        Calm,
        #[pgorm(string_value = "O'Brien 雪")]
        Busy,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity, Serialize, Deserialize)]
    #[pgorm(table_name = "accounts", schema_name = "fixture")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub tenant: i32,
        pub name: String,
        pub note: Option<String>,
        pub score: Option<i32>,
        pub rank: i32,
        pub active: bool,
        pub balance: Decimal,
        pub payload: Json,
        pub uuid: Uuid,
        pub created_at: DateTime,
        pub occurred_at: DateTimeWithTimeZone,
        pub event_date: Date,
        pub event_time: Time,
        pub state: State,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    #[async_trait::async_trait]
    impl ActiveModelBehavior for ActiveModel {
        async fn before_save<C>(mut self, _: &C, insert: bool) -> Result<Self, pgorm::Error>
        where
            C: pgorm::ConnectionTrait,
        {
            if insert {
                if let Some(name) = self.name.try_as_ref() {
                    self.name = pgorm::set(format!("{name}|hook"));
                }
            } else if let Some(rank) = self.rank.try_as_ref() {
                self.rank = pgorm::set(rank + 1);
            }
            Ok(self)
        }
    }
}

/// The notes table: the many side of the join, with a tenant column of its own
/// so a graph can leak across tenants without the relation noticing.
pub mod note {
    use pgorm::entity::prelude::*;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, Entity, Serialize, Deserialize)]
    #[pgorm(table_name = "notes", schema_name = "fixture")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub account_id: i32,
        pub tenant: i32,
        pub body: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// A room's rate over time, keyed `(id, valid_at WITHOUT OVERLAPS)`.
pub mod room {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "rooms", schema_name = "fixture")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(primary_key, without_overlaps)]
        pub valid_at: Range<Date>,
        pub rate: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// A guest's stay in a room.
pub mod stay {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "stays", schema_name = "fixture")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub room_id: i32,
        pub guest_id: i32,
        pub during: Range<Date>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// The registered graph shapes, by arity, each aliasing its joined sources with
/// caller-supplied names so hostile aliases reach the quoting path.
pub mod graphs {
    use super::{account, note, room, stay};

    use pgorm::{
        EntityTrait, Opt, RelationDef, Req, SelectGraph,
        pgorm_query::{Deferrability, Enforcement, Name},
    };

    pub fn relation() -> RelationDef {
        note::Entity::belongs_to(account::Entity)
            .columns(note::Column::AccountId, account::Column::Id)
            .into()
    }

    /// The root-only shape, taking the alias slice every other factory takes so
    /// that a generated call site does not special-case the one arity that
    /// joins nothing.
    pub fn account_only(_aliases: &[String]) -> SelectGraph<account::Entity, ()> {
        account::Entity::graph()
    }

    pub fn optional(aliases: &[String]) -> SelectGraph<account::Entity, (Opt<note::Entity>,)> {
        account::Entity::graph()
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[0]))
    }

    pub fn required(aliases: &[String]) -> SelectGraph<account::Entity, (Req<note::Entity>,)> {
        account::Entity::graph()
            .join_one_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[0]))
    }

    pub fn self_join(aliases: &[String]) -> SelectGraph<account::Entity, (Opt<account::Entity>,)> {
        let relation = account::Entity::belongs_to(account::Entity)
            .columns(account::Column::Rank, account::Column::Id)
            .into();
        account::Entity::graph()
            .join_maybe_as::<account::Entity>(relation, Name::runtime(&aliases[0]))
    }

    pub type Notes2 = (Opt<note::Entity>, Opt<note::Entity>);

    pub fn arity3(aliases: &[String]) -> SelectGraph<account::Entity, Notes2> {
        optional(&aliases[..1])
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[1]))
    }

    pub type Notes3 = (Opt<note::Entity>, Opt<note::Entity>, Opt<note::Entity>);

    pub fn arity4(aliases: &[String]) -> SelectGraph<account::Entity, Notes3> {
        optional(&aliases[..1])
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[1]))
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[2]))
    }

    pub type Notes4 = (
        Opt<note::Entity>,
        Opt<note::Entity>,
        Opt<note::Entity>,
        Opt<note::Entity>,
    );

    pub fn arity5(aliases: &[String]) -> SelectGraph<account::Entity, Notes4> {
        optional(&aliases[..1])
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[1]))
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[2]))
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[3]))
    }

    pub type Notes5 = (
        Opt<note::Entity>,
        Opt<note::Entity>,
        Opt<note::Entity>,
        Opt<note::Entity>,
        Opt<note::Entity>,
    );

    pub fn arity6(aliases: &[String]) -> SelectGraph<account::Entity, Notes5> {
        optional(&aliases[..1])
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[1]))
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[2]))
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[3]))
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[4]))
    }

    pub type Notes6 = (
        Opt<note::Entity>,
        Opt<note::Entity>,
        Opt<note::Entity>,
        Opt<note::Entity>,
        Opt<note::Entity>,
        Opt<note::Entity>,
    );

    pub fn arity7(aliases: &[String]) -> SelectGraph<account::Entity, Notes6> {
        optional(&aliases[..1])
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[1]))
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[2]))
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[3]))
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[4]))
            .join_maybe_as::<note::Entity>(relation().rev(), Name::runtime(&aliases[5]))
    }

    /// A stay's room: every version whose period overlaps the stay's.
    pub fn stay_room() -> RelationDef {
        stay::Entity::belongs_to(room::Entity)
            .columns(stay::Column::RoomId, room::Column::Id)
            .period(stay::Column::During, room::Column::ValidAt)
            .deferrability(Deferrability::DeferrableInitiallyDeferred)
            .into()
    }

    /// A stay's guest, an account the server never checks.
    pub fn stay_guest() -> RelationDef {
        stay::Entity::belongs_to(account::Entity)
            .columns(stay::Column::GuestId, account::Column::Id)
            .enforcement(Enforcement::NotEnforced)
            .into()
    }

    pub fn stay_rooms(aliases: &[String]) -> SelectGraph<stay::Entity, (Opt<room::Entity>,)> {
        stay::Entity::graph().join_maybe_as::<room::Entity>(stay_room(), Name::runtime(&aliases[0]))
    }

    pub fn stay_guests(aliases: &[String]) -> SelectGraph<stay::Entity, (Opt<account::Entity>,)> {
        stay::Entity::graph()
            .join_maybe_as::<account::Entity>(stay_guest(), Name::runtime(&aliases[0]))
    }
}
