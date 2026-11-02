use pgorm::entity::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, ActiveEnum)]
#[pgorm(schema_name = "sales")]
pub enum Stage {
    Lead,
    Won,
    Lost,
}
