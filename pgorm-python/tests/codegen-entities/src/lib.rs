#[path = "../../application-binding/src/account.rs"]
pub mod account;
pub mod courier;
/// The entity pgorm-codegen generates from a table of interval columns, so
/// the Python wrappers are generated over Rust codegen's own output.
#[path = "../../../../pgorm-codegen/tests/sql/interval/interval_span.rs"]
pub mod interval_span;
#[path = "../../application-binding/src/note.rs"]
pub mod note;
/// The enum, the range type's newtype and the entity pgorm-codegen generates
/// from a table of domain columns: each field types as its domain's base.
#[path = "../../../../pgorm-codegen/tests/sql/domain/pgorm_active_enums.rs"]
pub mod pgorm_active_enums;
#[path = "../../../../pgorm-codegen/tests/sql/domain/pgorm_range_types.rs"]
pub mod pgorm_range_types;
#[path = "../../../../pgorm-codegen/tests/sql/domain/stock_line.rs"]
pub mod stock_line;

use pgorm::pgorm_query::Name;
use pgorm::{EntityTrait, Opt, RelationDef, Req, SelectGraph};

impl pgorm::Related<account::Entity> for note::Entity {
    fn to() -> RelationDef {
        note::Entity::belongs_to(account::Entity)
            .columns(note::Column::AccountId, account::Column::Id)
            .into()
    }
}

pub fn account_only(_: &[String]) -> SelectGraph<account::Entity, ()> {
    account::Entity::graph()
}

pub fn optional(aliases: &[String]) -> SelectGraph<account::Entity, (Opt<note::Entity>,)> {
    account::Entity::graph().join_maybe_as::<note::Entity>(
        account::Entity::has_many(note::Entity).into(),
        Name::runtime(&aliases[0]),
    )
}

pub fn required(aliases: &[String]) -> SelectGraph<account::Entity, (Req<note::Entity>,)> {
    account::Entity::graph().join_one_as::<note::Entity>(
        account::Entity::has_many(note::Entity).into(),
        Name::runtime(&aliases[0]),
    )
}
