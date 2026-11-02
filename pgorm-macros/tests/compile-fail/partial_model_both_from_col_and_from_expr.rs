use pgorm::entity::prelude::*;

#[derive(PartialModel)]
#[pgorm(entity = "Entity")]
struct Partial {
    #[pgorm(from_col = "name", from_expr = "Expr::val(1)")]
    label: String,
}

fn main() {}
