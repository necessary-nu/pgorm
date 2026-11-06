//! A migrator built at run time.

use std::fmt;
use std::sync::Arc;

use pgorm::pgorm_query::IntoName;
use pgorm::{DatabaseTransaction, Name};

use crate::{Error, MigrationName, MigrationTrait, MigratorTrait, ledger};

/// A migrator built as the program runs rather than declared as a type: the
/// migrations it applies, in order, and where its ledger lives.
///
/// It is a [`MigratorTrait`] like any other, so [`up`](MigratorTrait::up),
/// [`status`](MigratorTrait::status) and every accessor run the one runner.
/// Its list can come from anywhere: migrations read from a directory with
/// [`SqlMigration::read_dir`](crate::SqlMigration::read_dir), migrations
/// written in Rust, or both.
///
/// ```
/// use pgorm_migration::prelude::*;
///
/// let migrations = Migrations::new(
///     SqlMigration::read_dir("tests/sql")?
///         .into_iter()
///         .map(|migration| Box::new(migration) as Box<dyn MigrationTrait>),
/// )
/// .with_migration_table_schema(Name::runtime("ops"));
/// let names: Vec<String> = migrations
///     .migrations()
///     .iter()
///     .map(|migration| migration.name().to_owned())
///     .collect();
/// assert_eq!(names, ["m20260101_000001_create_note", "m20260101_000002_seed_note"]);
/// # Ok::<(), std::io::Error>(())
/// ```
// [spec:pgorm:def:migration.runtime]
#[derive(Clone)]
pub struct Migrations {
    migrations: Vec<Arc<dyn MigrationTrait>>,
    table_name: Name,
    table_schema: Option<Name>,
    statement_cache_channel: Option<String>,
}

impl Migrations {
    /// A migrator applying `migrations` in the order given, its ledger
    /// placed and its channel named as a [`MigratorTrait`] that overrides
    /// nothing places and names them.
    pub fn new(migrations: impl IntoIterator<Item = Box<dyn MigrationTrait>>) -> Self {
        Self {
            migrations: migrations.into_iter().map(Arc::from).collect(),
            table_name: ledger::Entity.into_name(),
            table_schema: None,
            statement_cache_channel: Some(pgorm::STATEMENT_CACHE_CHANNEL.to_owned()),
        }
    }

    /// The ledger's table name, which
    /// [`migration_table_name`](MigratorTrait::migration_table_name) answers.
    pub fn with_migration_table_name(mut self, name: Name) -> Self {
        self.table_name = name;
        self
    }

    /// The ledger's schema, which
    /// [`migration_table_schema`](MigratorTrait::migration_table_schema)
    /// answers.
    pub fn with_migration_table_schema(mut self, schema: Name) -> Self {
        self.table_schema = Some(schema);
        self
    }

    /// The channel notified after a batch, which
    /// [`statement_cache_channel`](MigratorTrait::statement_cache_channel)
    /// answers; `None` notifies nothing.
    pub fn with_statement_cache_channel(mut self, channel: Option<String>) -> Self {
        self.statement_cache_channel = channel;
        self
    }
}

impl fmt::Debug for Migrations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Migrations")
            .field(
                "migrations",
                &self
                    .migrations
                    .iter()
                    .map(|migration| migration.name())
                    .collect::<Vec<_>>(),
            )
            .field("table_name", &self.table_name)
            .field("table_schema", &self.table_schema)
            .field("statement_cache_channel", &self.statement_cache_channel)
            .finish()
    }
}

// [spec:pgorm:def:migration.runtime]
impl MigratorTrait for Migrations {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        self.migrations
            .iter()
            .map(|migration| Box::new(Shared(migration.clone())) as Box<dyn MigrationTrait>)
            .collect()
    }

    fn migration_table_name(&self) -> Name {
        self.table_name.clone()
    }

    fn migration_table_schema(&self) -> Option<Name> {
        self.table_schema.clone()
    }

    fn statement_cache_channel(&self) -> Option<String> {
        self.statement_cache_channel.clone()
    }
}

/// One of a [`Migrations`]' migrations, handed out each time the runner asks
/// for the list.
struct Shared(Arc<dyn MigrationTrait>);

impl MigrationName for Shared {
    fn name(&self) -> &str {
        self.0.name()
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Shared {
    async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        self.0.up(tx).await
    }

    fn checksum(&self) -> Option<String> {
        self.0.checksum()
    }

    fn commits_after(&self) -> bool {
        self.0.commits_after()
    }
}
