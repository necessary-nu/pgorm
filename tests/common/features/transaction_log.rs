use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "transaction_log")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub date: Date,
    pub time: Time,
    pub date_time: DateTime,
    pub date_time_tz: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
