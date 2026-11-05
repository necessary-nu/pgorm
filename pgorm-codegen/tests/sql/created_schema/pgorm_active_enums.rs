use pgorm::entity::prelude::*;

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Debug, PartialEq, Eq, ActiveEnum)]
#[pgorm(schema_name = "sales")]
pub enum Stage {
    Lead,
    Won,
    Lost,
}
