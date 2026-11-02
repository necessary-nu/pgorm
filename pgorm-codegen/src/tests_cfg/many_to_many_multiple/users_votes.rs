use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "users_votes")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub user_id: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub bill_id: i32,
    pub user_idd: Option<i32> ,
    pub bill_idd: Option<i32> ,
    pub vote: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = "super::bills::Entity",
        from = "Column::BillIdd",
        to = "super::bills::Column::Id",
    )]
    Bills2,
    #[pgorm(
        belongs_to = "super::bills::Entity",
        from = "Column::BillId",
        to = "super::bills::Column::Id",
    )]
    Bills1,
    #[pgorm(
        belongs_to = "super::users::Entity",
        from = "Column::UserIdd",
        to = "super::users::Column::Id",
    )]
    Users2,
    #[pgorm(
        belongs_to = "super::users::Entity",
        from = "Column::UserId",
        to = "super::users::Column::Id",
    )]
    Users1,
}

impl ActiveModelBehavior for ActiveModel {}
