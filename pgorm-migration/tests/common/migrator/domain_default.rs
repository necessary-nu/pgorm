use pgorm_migration::prelude::*;
use pgorm_test_support::PREPARED_STATEMENT;

pub const INSERT: &str = "INSERT INTO line (id) VALUES ($1) RETURNING qty";

/// The [`INSERT`] `db`'s session holds prepared, or `None` once a clear has
/// closed it.
pub async fn prepared_insert(db: &impl ConnectionTrait) -> Result<Option<String>, Error> {
    Ok(db
        .query_opt(PREPARED_STATEMENT, &[&INSERT])
        .await?
        .map(|row| row.get(0)))
}

/// A migration that runs one script, its name pinned by hand, and asks to be
/// committed before the next one runs where `commits_after` says so.
pub struct Script {
    pub name: &'static str,
    pub sql: &'static str,
    pub commits_after: bool,
}

impl MigrationName for Script {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Script {
    async fn up(&self, tx: &DatabaseTransaction<'_>) -> Result<(), Error> {
        tx.batch_execute(self.sql).await
    }

    fn commits_after(&self) -> bool {
        self.commits_after
    }
}

pub const CREATED: Script = Script {
    name: "m20240201_000001_create_quantity",
    sql: "CREATE DOMAIN quantity AS integer DEFAULT 1; \
          CREATE TABLE line (id integer PRIMARY KEY, qty quantity)",
    commits_after: false,
};

pub const DEFAULTED: Script = Script {
    name: "m20240201_000002_default_quantity",
    sql: "ALTER DOMAIN quantity SET DEFAULT 2",
    commits_after: false,
};

/// A domain with a default, and a table with a column of it.
pub struct Created;

#[async_trait::async_trait]
impl MigratorTrait for Created {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CREATED)]
    }
}

/// [`Created`], then a migration changing the domain's default.
pub struct Defaulted;

#[async_trait::async_trait]
impl MigratorTrait for Defaulted {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CREATED), Box::new(DEFAULTED)]
    }
}

pub const REDEFAULTED: Script = Script {
    name: "m20240201_000003_redefault_quantity",
    sql: "ALTER DOMAIN quantity SET DEFAULT 3",
    commits_after: false,
};

/// [`Defaulted`], then another change to the default, from a migrator that
/// notifies no channel.
pub struct Unannounced;

#[async_trait::async_trait]
impl MigratorTrait for Unannounced {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(CREATED),
            Box::new(DEFAULTED),
            Box::new(REDEFAULTED),
        ]
    }

    fn statement_cache_channel() -> Option<String> {
        None
    }
}
