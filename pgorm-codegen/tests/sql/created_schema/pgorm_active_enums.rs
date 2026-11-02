use pgorm::entity::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, ActiveEnum)]
#[pgorm(
    rs_type = String,
    enum_name = "stage",
    schema_name = "sales"
)]
pub enum Stage {
    #[pgorm(string_value = "lead")]
    Lead,
    #[pgorm(string_value = "won")]
    Won,
    #[pgorm(string_value = "lost")]
    Lost,
}
