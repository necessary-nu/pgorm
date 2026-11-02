use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "bills")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub user_id: Option<i32> ,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = "super::users::Entity",
        from = "Column::UserId",
        to = "super::users::Column::Id",
        on_update = "NoAction",
        on_delete = "NoAction",
    )]
    Users,
}

impl Related<super::users::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Users.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
