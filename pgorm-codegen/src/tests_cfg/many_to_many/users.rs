use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "users")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::Text)]
    pub email: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::bills::Entity, related = false)]
    Bills,
    #[pgorm(has_many = super::users_votes::Entity)]
    UsersVotes,
}

impl Related<super::bills::Entity> for Entity {
    fn to() -> RelationDef {
        super::users_votes::Relation::Bills.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::users_votes::Relation::Users.def().rev())
    }
}
