use pgorm::entity::prelude::*;

#[derive(Debug, ActiveEnum)]
#[pgorm(db_type = ColumnType::Integer)]
pub enum Missing {
    #[pgorm(num_value = 1)]
    One,
}

fn main() {}
