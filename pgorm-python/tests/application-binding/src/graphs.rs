use crate::{account, membership, note, room, stay};

use pgorm::pgorm_query::Name;
use pgorm::{EntityTrait, Opt, RelationDef, RelationTrait, Req, SelectGraph};
use pyo3::prelude::*;

pub fn relation() -> RelationDef {
    account::Entity::has_many(note::Entity).into()
}

impl pgorm::Related<account::Entity> for note::Entity {
    fn to() -> RelationDef {
        note::Entity::belongs_to(account::Entity)
            .columns(note::Column::AccountId, account::Column::Id)
            .into()
    }
}

pub fn optional(aliases: &[String]) -> SelectGraph<account::Entity, (Opt<note::Entity>,)> {
    account::Entity::graph().join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[0]))
}

pub fn required(aliases: &[String]) -> SelectGraph<account::Entity, (Req<note::Entity>,)> {
    account::Entity::graph().join_one_as::<note::Entity>(relation(), Name::runtime(&aliases[0]))
}

/// A stay and every version of its room whose period overlaps the stay's: the
/// temporal relation joins by `&&` beside the key's equality.
pub fn stay_rooms(aliases: &[String]) -> SelectGraph<stay::Entity, (Opt<room::Entity>,)> {
    stay::Entity::graph()
        .join_maybe_as::<room::Entity>(stay::Relation::Room.def(), Name::runtime(&aliases[0]))
}

// [spec:pgorm:req:bindings.entity-graphs+4/test]
pub fn register(registry: &mut pgorm_python::entities::Registry) -> PyResult<()> {
    registry.graph::<account::Entity, (), _>("app.AccountOnly", |_| account::Entity::graph())?;
    registry.graph::<membership::Entity, (), _>("app.MembershipOnly", |_| {
        membership::Entity::graph()
    })?;
    registry.graph("app.AccountNotes", optional)?;
    registry.graph("app.StayRooms", stay_rooms)?;
    registry.graph("app.StayGuest", |aliases| {
        stay::Entity::graph().join_maybe_as::<account::Entity>(
            stay::Relation::Guest.def(),
            Name::runtime(&aliases[0]),
        )
    })?;
    registry.graph("app.StayGuestRequired", |aliases| {
        stay::Entity::graph()
            .join_one_as::<account::Entity>(stay::Relation::Guest.def(), Name::runtime(&aliases[0]))
    })?;
    registry.graph("app.RequiredNotes", required)?;
    registry.graph("app.MixedNotes", |aliases| {
        required(&aliases[..1])
            .join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[1]))
    })?;
    registry.graph("app.FourSources", |aliases| {
        optional(&aliases[..1])
            .join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[1]))
            .join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[2]))
    })?;
    registry.graph("app.FiveSources", |aliases| {
        optional(&aliases[..1])
            .join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[1]))
            .join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[2]))
            .join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[3]))
    })?;
    registry.graph("app.SixSources", |aliases| {
        optional(&aliases[..1])
            .join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[1]))
            .join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[2]))
            .join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[3]))
            .join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[4]))
    })?;
    registry.graph("app.SevenSources", |aliases| {
        optional(&aliases[..1])
            .join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[1]))
            .join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[2]))
            .join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[3]))
            .join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[4]))
            .join_maybe_as::<note::Entity>(relation(), Name::runtime(&aliases[5]))
    })?;
    Ok(())
}
