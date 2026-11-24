use pgorm::entity::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueType, Scope)]
pub struct UserId(pub i64);

mod optional {
    use super::UserId;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "optional")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i64,
        #[pgorm(scope)]
        pub author_id: Option<UserId>,
    }
}

mod twice {
    use super::UserId;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "twice")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i64,
        #[pgorm(scope, scope)]
        pub author_id: UserId,
    }
}

mod no_scopes_written {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "no_scopes_written", scope())]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i64,
    }
}

#[derive(Clone, Copy, Debug, ScopeContext)]
pub struct Pair {
    pub author: UserId,
    pub reader: Every<UserId>,
}

#[derive(Clone, Copy, Debug, ScopeContext)]
pub struct Generic<T> {
    pub user: T,
}

#[derive(Clone, Copy, Debug, ScopeContext)]
pub enum NotAStruct {
    User(UserId),
}

fn main() {}
