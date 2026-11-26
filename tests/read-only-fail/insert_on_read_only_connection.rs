use pgorm::entity::prelude::*;
use pgorm::tests_cfg::cake;

async fn add(db: &ReadOnlyConnection) -> Result<(), Error> {
    cake::ActiveModel {
        name: set("Lemon"),
        ..Default::default()
    }
    .insert(db)
    .await?;
    Ok(())
}

fn main() {
    let _ = add;
}
