use pgorm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, EnumIter, ActiveEnum, Serialize, Deserialize)]
#[pgorm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "State\" 雪",
    schema_name = "fixture"
)]
pub enum State {
    #[pgorm(string_value = "calm")]
    Calm,
    #[pgorm(string_value = "O'Brien 雪")]
    Busy,
}

// [spec:pgorm:req:generative.execution]
#[derive(Clone, Debug, PartialEq, Eq, Entity, Serialize, Deserialize)]
#[pgorm(table_name = "accounts", schema_name = "fixture")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub tenant: i32,
    pub name: String,
    pub note: Option<String>,
    pub score: Option<i32>,
    pub rank: i32,
    pub active: bool,
    pub balance: Decimal,
    pub payload: Json,
    pub uuid: Uuid,
    pub created_at: DateTime,
    pub occurred_at: DateTimeWithTimeZone,
    pub event_date: Date,
    pub event_time: Time,
    pub state: State,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    async fn before_save<C>(mut self, _: &C, insert: bool) -> Result<Self, pgorm::Error>
    where
        C: pgorm::ConnectionTrait,
    {
        if insert {
            if let Some(name) = self.name.try_as_ref() {
                self.name = pgorm::set(format!("{name}|hook"));
            }
        } else if let Some(rank) = self.rank.try_as_ref() {
            self.rank = pgorm::set(rank + 1);
        }
        Ok(self)
    }
}
