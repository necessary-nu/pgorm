use pgorm::{Connection, Error, ReadOnlyConnection, sql};

async fn purge(db: &ReadOnlyConnection) -> Result<u64, Error> {
    db.execute(sql!("WITH gone AS (DELETE FROM cake RETURNING id) SELECT count(*) FROM gone"), &[])
        .await
}

fn main() {
    let _ = purge;
}
