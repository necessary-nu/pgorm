use pgorm::entity::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, ActiveEnum)]
pub enum Mood {
    Sad,
    Calm,
    Glad,
}
