use pgorm::entity::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, ActiveEnum)]
#[pgorm(
    rs_type = String,
    enum_name = "tier",
    schema_name = "crm"
)]
pub enum Tier {
    #[pgorm(string_value = "free")]
    Free,
    #[pgorm(string_value = "pro")]
    Pro,
    #[pgorm(string_value = "team")]
    Team,
}
#[derive(Debug, Clone, PartialEq, Eq, EnumIter, ActiveEnum)]
#[pgorm(rs_type = String, enum_name = "mood")]
pub enum Mood {
    #[pgorm(string_value = "sad")]
    Sad,
    #[pgorm(string_value = "calm")]
    Calm,
    #[pgorm(string_value = "glad")]
    Glad,
}
