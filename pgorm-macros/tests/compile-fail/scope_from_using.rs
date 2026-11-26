use pgorm::entity::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueType, Scope)]
pub struct CompanyId(pub i64);

mod company {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "company")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i64,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod note {
    use super::CompanyId;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "note")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i64,
        #[pgorm(scope)]
        pub company_id: CompanyId,
        pub body: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

fn main() {
    let _ = Update::many(company::Entity).from(note::Entity);
    let _ = Delete::many(company::Entity).using(note::Entity);
}
