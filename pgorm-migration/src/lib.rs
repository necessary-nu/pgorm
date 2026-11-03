#![deny(rust_2018_idioms)]

pub mod ledger;
pub mod migrator;
pub mod prelude;
mod sql_file;
pub mod util;

pub use migrator::*;
pub use sql_file::SqlMigration;

#[doc(hidden)]
pub mod __private {
    pub use pgorm_sql_macro::sql_migration;
}

pub use async_trait;
pub use pgorm;
use pgorm::DatabaseTransaction;
pub use pgorm::Error;
pub use pgorm::pgorm_query;

// [spec:pgorm:sem:migration.name+4]    ledger identity, normally the file stem
pub trait MigrationName {
    fn name(&self) -> &str;
}

/// Implement [`MigrationName`] for a migration, naming it for the file stem of
/// the file it is written in. The derive and the trait share a name and live
/// in different namespaces.
// [spec:pgorm:req:macros.derive.names+2]
pub use pgorm_macros::MigrationName;

/// The migration definition
// [spec:pgorm:def:migration.runner+4]    author-facing half
// [spec:pgorm:req:migration.up-only]    `up` is the only direction
#[async_trait::async_trait]
#[doc(alias = "SchemaManager")]
pub trait MigrationTrait: MigrationName + Send + Sync {
    /// Define actions to perform when applying the migration
    async fn up(&self, tx: &DatabaseTransaction<'_>) -> Result<(), Error>;

    /// An opaque digest of what this migration does, stored in the ledger when
    /// the migration is applied and compared against on every later run.
    ///
    /// The default is `None`: content drift is not detected for a migration
    /// that does not opt in. There is no derived answer because nothing the
    /// crate can see is a faithful stand-in for "what this migration does" —
    /// the name is already the ledger key, and hashing the source file would
    /// make a reformatted comment look like a schema change. Overriding this
    /// with a value the author controls — a hash of the DDL text, a version
    /// string bumped by hand — is what turns the check on.
    // [spec:pgorm:req:migration.checksum+1]    opt-in by override, `None` by default
    fn checksum(&self) -> Option<String> {
        None
    }

    /// Whether [`up`](MigratorTrait::up) commits this migration, with its
    /// ledger row, before the next migration of the batch runs, `false` by
    /// default.
    ///
    /// Answering `true` is for a change PostgreSQL will not let one
    /// transaction finish: a value `ALTER TYPE ... ADD VALUE` adds cannot be
    /// used before the transaction that added it commits. Such a batch is not
    /// all-or-nothing: a failure after the commit rolls back only the
    /// transaction it happens in, the migrations committed before it stay
    /// applied, and running `up` again resumes after them.
    // [spec:pgorm:req:migration.commit-after]
    fn commits_after(&self) -> bool {
        false
    }
}
