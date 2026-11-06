use migration::Migrator;
use pgorm_migration::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt::init();

    let url = std::env::var("DATABASE_URL").expect("Environment variable 'DATABASE_URL' not set");
    let db = pgorm::connect(url.parse().expect("DATABASE_URL is not a valid connection string"));

    match std::env::args().nth(1).as_deref() {
        None | Some("up") => {
            let steps = std::env::args()
                .nth(2)
                .map(|s| s.parse().expect("step count must be a number"));
            Migrator.up(db, steps).await?;
        }
        Some("status") => {
            Migrator.status(&db.get().await?).await?;
        }
        Some(other) => {
            eprintln!("unknown command '{other}'; expected 'up [n]' or 'status'");
            std::process::exit(1);
        }
    }

    Ok(())
}
