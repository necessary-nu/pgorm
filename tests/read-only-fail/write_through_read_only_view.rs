use pgorm::entity::prelude::*;
use pgorm::tests_cfg::cake;

async fn add(db: &DatabaseConnection) -> Result<(), Error> {
    let reader = db.read_only();
    Insert::one(cake::ActiveModel {
        name: set("Lemon"),
        ..Default::default()
    })
    .exec(&reader)
    .await?;
    Ok(())
}

fn main() {
    let _ = add;
}
