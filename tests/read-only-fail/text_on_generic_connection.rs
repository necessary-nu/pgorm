use pgorm::{Connection, Error};

async fn purge(db: &impl Connection) -> Result<u64, Error> {
    db.execute("DELETE FROM cake", &[]).await
}

fn main() {
    let _ = purge::<pgorm::DatabaseConnection>;
}
