use crate::common::migrator::default;
use pgorm_migration::prelude::*;

/// The schema [`Separate`] keeps its ledger in. It is mixed-case so that a
/// lookup which fails to quote it folds it to `ledger` and misses.
pub const LEDGER_SCHEMA: &str = "Ledger";

/// The default migrations, their ledger in a schema of its own.
pub struct Separate;

#[async_trait::async_trait]
impl MigratorTrait for Separate {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        default::Migrator.migrations()
    }

    fn migration_table_schema(&self) -> Option<Name> {
        Some(Name::runtime(LEDGER_SCHEMA))
    }
}
