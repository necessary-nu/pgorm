//! An application's entity crate, apart from any binding: its entities and
//! the graph shapes it builds from them. pgorm-napi's codegen writes the
//! native module registering them and the TypeScript typing them.

#[path = "../../application-binding/src/account.rs"]
pub mod account;
pub mod courier;
#[path = "../../application-binding/src/note.rs"]
pub mod note;
pub mod sample;

use pgorm::pgorm_query::Name;
use pgorm::{EntityTrait, Opt, RelationTrait, Req, SelectGraph};

fn alias(aliases: &[String], at: usize) -> Name {
    Name::runtime(aliases.get(at).map_or("note", String::as_str))
}

/// Accounts alone.
pub fn account_only(_: &[String]) -> SelectGraph<account::Entity, ()> {
    account::Entity::graph()
}

/// Each account with a note, or none.
pub fn optional(aliases: &[String]) -> SelectGraph<account::Entity, (Opt<note::Entity>,)> {
    account::Entity::graph()
        .join_maybe_as::<note::Entity>(account::Relation::Note.def(), alias(aliases, 0))
}

/// Each account with a note it must have, and another it may.
pub fn mixed(
    aliases: &[String],
) -> SelectGraph<account::Entity, (Req<note::Entity>, Opt<note::Entity>)> {
    account::Entity::graph()
        .join_one_as::<note::Entity>(account::Relation::Note.def(), alias(aliases, 0))
        .join_maybe_as::<note::Entity>(account::Relation::Note.def(), alias(aliases, 1))
}
