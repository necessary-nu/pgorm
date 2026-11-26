use pgorm::entity::prelude::*;
use pgorm::tests_cfg::cake;

async fn rename<C: Connection>(db: &C) -> Result<(), Error> {
    Update::many(cake::Entity)
        .col_expr(cake::Column::Name, Expr::value("Lemon"))
        .exec(db)
        .await?;
    Ok(())
}

fn main() {
    let _ = rename::<DatabaseConnection>;
}
