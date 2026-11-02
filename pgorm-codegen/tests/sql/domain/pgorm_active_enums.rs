use pgorm::entity::prelude::*;

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
