use pgorm::entity::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[pgorm(rs_type = "String", db_type = "Enum", enum_name = "grade")]
pub enum Grade {
    #[pgorm(string_value = "low")]
    Low,
    #[pgorm(string_value = "high")]
    High,
}
