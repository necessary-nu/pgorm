use pgorm::entity::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, ActiveEnum)]
#[pgorm(schema_name = "crm")]
pub enum Tier {
    Free,
    Pro,
    Team,
}
#[derive(Debug, Clone, PartialEq, Eq, ActiveEnum)]
pub enum Mood {
    Sad,
    Calm,
    Glad,
}
