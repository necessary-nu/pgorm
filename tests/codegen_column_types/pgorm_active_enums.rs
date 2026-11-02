use pgorm::entity::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, ActiveEnum)]
#[pgorm(rs_type = String, enum_name = "grade")]
pub enum Grade {
    #[pgorm(string_value = "low")]
    Low,
    #[pgorm(string_value = "high")]
    High,
}
