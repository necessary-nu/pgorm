use pgorm::pgorm_query::{CopyFromStatement, CopyText, Name};
use pgorm::{CopyConnection, Error, ReadOnlyConnection};

async fn load(db: &mut ReadOnlyConnection) -> Result<u64, Error> {
    let copy = CopyFromStatement::new(Name::runtime("cake"), CopyText);
    db.copy_in(&copy).await?.finish().await
}

fn main() {
    let _ = load;
}
