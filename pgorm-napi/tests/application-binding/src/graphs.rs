//! The graph shapes the application registers: each one's root and slots
//! fixed here, its joins made under the aliases JavaScript chooses.

use pgorm::pgorm_query::Name;
use pgorm::{EntityTrait, Opt, RelationTrait, Req, SelectGraph};
use pgorm_napi::{RegistrationError, Registry};

use crate::{account, membership, note};

fn notes(aliases: &[String], at: usize) -> Name {
    Name::runtime(aliases.get(at).map_or("note", String::as_str))
}

pub fn optional(aliases: &[String]) -> SelectGraph<account::Entity, (Opt<note::Entity>,)> {
    account::Entity::graph()
        .join_maybe_as::<note::Entity>(account::Relation::Note.def(), notes(aliases, 0))
}

pub fn required(aliases: &[String]) -> SelectGraph<account::Entity, (Req<note::Entity>,)> {
    account::Entity::graph()
        .join_one_as::<note::Entity>(account::Relation::Note.def(), notes(aliases, 0))
}

/// Register every shape, once its source entities are registered.
// [spec:pgorm:req:bindings.entity-graphs+4/test]
pub fn register(registry: &mut Registry) -> Result<(), RegistrationError> {
    registry.graph::<account::Entity, (), _>("app.AccountOnly", |_| account::Entity::graph())?;
    registry.graph::<membership::Entity, (), _>("app.MembershipOnly", |_| {
        membership::Entity::graph()
    })?;
    registry.graph("app.AccountNotes", optional)?;
    registry.graph("app.RequiredNotes", required)?;
    registry.graph("app.MixedNotes", |aliases| {
        required(aliases)
            .join_maybe_as::<note::Entity>(account::Relation::Note.def(), notes(aliases, 1))
    })?;
    Ok(())
}
