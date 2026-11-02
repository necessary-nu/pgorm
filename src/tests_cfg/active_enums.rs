use crate as pgorm;
use crate::entity::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, ActiveEnum)]
#[pgorm(rs_type = String, enum_name = "tea")]
pub enum Tea {
    #[pgorm(string_value = "EverydayTea")]
    EverydayTea,
    #[pgorm(string_value = "BreakfastTea")]
    BreakfastTea,
}
