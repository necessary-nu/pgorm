use pgorm_migration::pgorm::{
    ConnectionTrait, DatabasePool, Error, ManagerConfig, NoTls, STATEMENT_CACHE_CHANNEL,
    types::ToSql,
};

use pgorm_test_support::{config, create_database, drop_database, scoped_name};

const NO_PARAMS: [&(dyn ToSql + Sync); 0] = [];

/// A throwaway database of this process's own, created on construction under
/// a name no other run can hold (`pgorm_test_support`).
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
            std::env::var("DATABASE_URL").expect("Environment variable 'DATABASE_URL' not set");
        let db_name = scoped_name(test_name);

        create_database(&base_url, &db_name)
            .await
            .expect("create test database");

        let db = pgorm_migration::pgorm::connect(config(&base_url, &db_name));

        Self {
            base_url,
            db_name,
            db,
        }
    }

    /// A second, independent pool against the same database — what a second
    /// deploy replica racing this one would hold.
    pub fn second_pool(&self) -> DatabasePool {
        pgorm_migration::pgorm::connect(config(&self.base_url, &self.db_name))
    }

    /// Another process's pool of one connection, listening on the channel the
    /// runner notifies by default.
    pub fn listening_pool(&self) -> DatabasePool {
        pgorm_migration::pgorm::connect_with(
            config(&self.base_url, &self.db_name),
            NoTls,
            ManagerConfig {
                statement_cache_channel: Some(STATEMENT_CACHE_CHANNEL.to_owned()),
                ..ManagerConfig::default()
            },
            |builder| builder.max_size(1),
        )
        .expect("a one-connection pool builds")
    }

    /// A pool against this database that lends one connection, so each
    /// checkout is the session the last one returned.
    pub fn pool_of_one(&self) -> DatabasePool {
        pgorm_migration::pgorm::connect_with_builder(
            config(&self.base_url, &self.db_name),
            |builder| builder.max_size(1),
        )
        .expect("a one-connection pool builds")
    }

    /// A pool against this database whose sessions act as `role`, which the
    /// server's superuser login is allowed to become.
    pub fn pool_as(&self, role: &str) -> DatabasePool {
        let mut config = config(&self.base_url, &self.db_name);
        config.options(format!("-c role={role}"));
        pgorm_migration::pgorm::connect(config)
    }

    /// Create a role for `label`, without login and without any privilege a
    /// test does not grant it, and answer its name. Roles belong to the cluster
    /// rather than to the test database, so the name is this run's own
    /// (`testing.isolation`), and [`TestContext::drop_role`] removes it again.
    pub async fn create_role(&self, label: &str) -> String {
        let role = scoped_name(label);
        let maintenance = pgorm_migration::pgorm::connect(config(&self.base_url, "postgres"));
        let conn = maintenance.get().await.expect("connect to maintenance db");
        conn.execute_raw(&format!("CREATE ROLE \"{role}\" NOLOGIN"), NO_PARAMS)
            .await
            .expect("create test role");
        role
    }

    /// Drop a role [`TestContext::create_role`] made, once the database whose
    /// objects it owned is gone.
    pub async fn drop_role(&self, role: &str) {
        let maintenance = pgorm_migration::pgorm::connect(config(&self.base_url, "postgres"));
        let conn = maintenance.get().await.expect("connect to maintenance db");
        let _ = conn
            .execute_raw(&format!("DROP ROLE IF EXISTS \"{role}\""), NO_PARAMS)
            .await;
    }

    /// Drop the database. A failure is left to the sweep a later run makes once
    /// this process has exited.
    pub async fn delete(&self) {
        if let Err(err) = drop_database(&self.base_url, &self.db_name).await {
            tracing::warn!("test database {:?} outlived its test: {err}", self.db_name);
        }
    }
}

pub async fn has_table(db: &DatabasePool, name: &str) -> Result<bool, Error> {
    let conn = db.get().await?;
    let params: [&(dyn ToSql + Sync); 1] = [&name];
    let row = conn
        .query_one("SELECT to_regclass($1) IS NOT NULL", &params)
        .await?;
    Ok(row.get(0))
}

pub async fn has_index(db: &DatabasePool, table: &str, index: &str) -> Result<bool, Error> {
    let conn = db.get().await?;
    let params: [&(dyn ToSql + Sync); 2] = [&table, &index];
    let row = conn
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE tablename = $1 AND indexname = $2)",
            &params,
        )
        .await?;
    Ok(row.get(0))
}

pub async fn has_column(db: &DatabasePool, table: &str, column: &str) -> Result<bool, Error> {
    let conn = db.get().await?;
    let params: [&(dyn ToSql + Sync); 2] = [&table, &column];
    let row = conn
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = $1 AND column_name = $2)",
            &params,
        )
        .await?;
    Ok(row.get(0))
}

pub async fn count_rows(db: &DatabasePool, table: &str) -> Result<i64, Error> {
    let conn = db.get().await?;
    let row = conn
        .query_one(&format!("SELECT COUNT(*) FROM \"{table}\""), &[])
        .await?;
    Ok(row.get(0))
}

pub async fn has_schema(db: &DatabasePool, schema: &str) -> Result<bool, Error> {
    let conn = db.get().await?;
    let params: [&(dyn ToSql + Sync); 1] = [&schema];
    let row = conn
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = $1)",
            &params,
        )
        .await?;
    Ok(row.get(0))
}

/// [`has_column`] for a table named in `schema` rather than the current one.
pub async fn has_column_in(
    db: &DatabasePool,
    schema: &str,
    table: &str,
    column: &str,
) -> Result<bool, Error> {
    let conn = db.get().await?;
    let params: [&(dyn ToSql + Sync); 3] = [&schema, &table, &column];
    let row = conn
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
             WHERE table_schema = $1 AND table_name = $2 AND column_name = $3)",
            &params,
        )
        .await?;
    Ok(row.get(0))
}

pub async fn count_rows_in(db: &DatabasePool, schema: &str, table: &str) -> Result<i64, Error> {
    let conn = db.get().await?;
    let row = conn
        .query_one(
            &format!("SELECT COUNT(*) FROM \"{schema}\".\"{table}\""),
            &[],
        )
        .await?;
    Ok(row.get(0))
}
