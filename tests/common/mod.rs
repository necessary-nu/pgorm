pub mod bakery_chain;
pub mod features;
pub mod runtime;
pub mod setup;

use pgorm::DatabasePool;

pub struct TestContext {
    base_url: String,
    db_name: String,
    pub db: DatabasePool,
}

impl TestContext {
    pub async fn new(test_name: &str) -> Self {
        dotenvy::from_filename(".env.local").ok();
        dotenvy::from_filename(".env").ok();

        let base_url =
            std::env::var("DATABASE_URL").expect("Enviroment variable 'DATABASE_URL' not set");
        let db_name = setup::scoped_db_name(test_name);
        let db: DatabasePool = setup::setup(&base_url, &db_name).await;

        Self {
            base_url,
            db_name,
            db,
        }
    }

    /// The database this context provisioned, for a test that opens a second
    /// pool of its own against it.
    pub fn db_name(&self) -> &str {
        &self.db_name
    }

    /// The configuration of the database this context provisioned, for a
    /// test that builds a pool of another kind against it.
    pub fn config(&self) -> pgorm::Config {
        setup::config(&self.base_url, &self.db_name)
    }

    /// Drop the database, taking the context by value so its pool — and every
    /// connection idling in it — is closed before the server is asked to drop
    /// the database out from under it.
    pub async fn delete(self) {
        let Self {
            base_url,
            db_name,
            db,
        } = self;
        drop(db);

        setup::tear_down(&base_url, &db_name).await;
    }
}
