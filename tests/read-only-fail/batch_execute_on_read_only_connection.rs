use pgorm::{Connection, Error, ReadOnlyConnection};

async fn drop_all(db: &ReadOnlyConnection) -> Result<(), Error> {
    db.batch_execute("DROP TABLE cake").await
}

fn main() {
    let _ = drop_all;
}
