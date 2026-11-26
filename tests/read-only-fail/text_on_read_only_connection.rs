use pgorm::{Connection, Error, ReadOnlyConnection};

async fn count(db: &ReadOnlyConnection, table: &str) -> Result<i64, Error> {
    let sql = format!("SELECT count(*) FROM {table}");
    Ok(db.query_one(&sql, &[]).await?.get(0))
}

fn main() {
    let _ = count;
}
