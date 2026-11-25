use pgorm::entity::prelude::*;
use pgorm::{NoContext, ScopedIn};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueType, Scope)]
pub struct CompanyId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueType, Scope)]
pub struct UserId(pub i64);

#[derive(Clone, Copy, Debug, ScopeContext)]
pub struct Visitor {
    pub company: CompanyId,
}

mod note {
    use super::{CompanyId, UserId};
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "note")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i64,
        #[pgorm(scope)]
        pub company_id: CompanyId,
        #[pgorm(scope)]
        pub author_id: UserId,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// What every `_for` form asks of its entity under the context it is given.
fn built_under<E: ScopedIn<Cx>, Cx: ScopeContext>(_: &Cx) {}

/// What every form taking no context asks of its entity.
fn built_plain<E: ScopedIn<NoContext>>() {}

fn main() {
    built_under::<note::Entity, _>(&CompanyId(7));
    built_under::<note::Entity, _>(&Visitor {
        company: CompanyId(7),
    });
    built_plain::<note::Entity>();
}
