use pgorm::entity::prelude::*;

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Eq, ActiveEnum)]
#[pgorm(schema_name = "sales")]
pub enum Stage {
    Lead,
    Won,
    Lost,
}
