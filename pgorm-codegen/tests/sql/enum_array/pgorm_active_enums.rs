use pgorm::entity::prelude::*;

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Eq, ActiveEnum)]
#[pgorm(schema_name = "crm")]
pub enum Tier {
    Free,
    Pro,
    Team,
}
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Eq, ActiveEnum)]
pub enum Mood {
    Sad,
    Calm,
    Glad,
}
