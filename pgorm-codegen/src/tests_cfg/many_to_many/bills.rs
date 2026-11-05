use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "bills")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub user_id: Option<i32>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::users::Entity,
        from = Column::UserId,
        to = super::users::Column::Id,
        on_update = ForeignKeyAction::NoAction,
        on_delete = ForeignKeyAction::NoAction,
        related = false
    )]
    Users,
    #[pgorm(has_many = super::users_votes::Entity)]
    UsersVotes,
}

impl Related<super::users::Entity> for Entity {
    fn to() -> RelationDef {
        super::users_votes::Relation::Users.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::users_votes::Relation::Bills.def().rev())
    }
}
