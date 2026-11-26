use pgorm::entity::prelude::*;
use pgorm::tests_cfg::cake;
use pgorm::TransactionError;

async fn purge(db: &mut DatabaseConnection) -> Result<(), TransactionError<Error>> {
    db.read_only_transaction(None, async |tx| {
        Delete::many(cake::Entity).exec(tx).await?;
        Ok(())
    })
    .await
}

fn main() {
    let _ = purge;
}
