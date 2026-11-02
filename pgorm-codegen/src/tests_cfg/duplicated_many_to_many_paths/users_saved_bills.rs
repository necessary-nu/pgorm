use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "users_saved_bills")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub user_id: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub bill_id: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::bills::Entity,
        from = Column::BillId,
        to = super::bills::Column::Id,
        on_update = ForeignKeyAction::Cascade,
        on_delete = ForeignKeyAction::Cascade,
    )]
    Bills,
    #[pgorm(
        belongs_to = super::users::Entity,
        from = Column::UserId,
        to = super::users::Column::Id,
        on_update = ForeignKeyAction::Cascade,
        on_delete = ForeignKeyAction::Cascade,
    )]
    Users,
}

impl Related<super::bills::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Bills.def()
    }
}

impl Related<super::users::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Users.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
