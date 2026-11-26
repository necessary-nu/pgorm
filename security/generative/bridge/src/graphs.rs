use crate::{account, note, room, stay};

use pgorm::pgorm_query::{Deferrability, Enforcement, Name};
use pgorm::{Entity, Opt, RelationDef, Req, SelectGraph};

pub fn relation() -> RelationDef {
    note::Entity::belongs_to(account::Entity)
        .columns(note::Column::AccountId, account::Column::Id)
        .into()
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
    account::Entity::graph().join_maybe_as::<account::Entity>(relation, Name::runtime(&aliases[0]))
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

/// A stay's room: every version whose period overlaps the stay's, a temporal
/// foreign key the server checks at commit.
pub fn stay_room() -> RelationDef {
    stay::Entity::belongs_to(room::Entity)
        .columns(stay::Column::RoomId, room::Column::Id)
        .period(stay::Column::During, room::Column::ValidAt)
        .deferrability(Deferrability::DeferrableInitiallyDeferred)
        .into()
}

/// A stay's guest, an account the server records and never checks, so a stay
/// may name one that does not exist.
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
    stay::Entity::graph().join_maybe_as::<account::Entity>(stay_guest(), Name::runtime(&aliases[0]))
}
