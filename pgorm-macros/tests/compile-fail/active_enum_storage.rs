use pgorm::entity::prelude::*;

#[derive(Debug, ActiveEnum)]
#[pgorm(rs_type = String)]
pub enum LoneRsType {
    Big,
}

#[derive(Debug, ActiveEnum)]
pub enum NumberedPostgresEnum {
    #[pgorm(num_value = 1)]
    One,
}

#[derive(Debug, ActiveEnum)]
pub enum DiscriminatedPostgresEnum {
    One = 1,
}

#[derive(Debug, ActiveEnum)]
#[pgorm(enum_nmae = "tea")]
pub enum UnknownContainerKey {
    Big,
}

#[derive(Debug, ActiveEnum)]
pub enum UnknownVariantKey {
    #[pgorm(string_valeu = "big")]
    Big,
}

fn main() {}
