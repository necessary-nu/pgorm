use pgorm::entity::prelude::*;

// `display_value` is the text the opt-in `display` writes; without `display`
// nothing would read it.
#[derive(Debug, EnumIter, ActiveEnum)]
#[pgorm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum Size {
    #[pgorm(string_value = "S", display_value = "Small")]
    Small,
}

fn main() {}
