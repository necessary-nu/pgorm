use pgorm::entity::prelude::*;

// [spec:pgorm:def:migration.runner+5]    ledger schema and default name
// [spec:pgorm:req:migration.checksum+1]    the nullable third column
#[derive(Clone, Debug, PartialEq, Eq, Entity)]
// The default name only: a migrator places its ledger through
// `MigratorTrait::migration_table_name` and `MigratorTrait::migration_table_schema`
#[pgorm(table_name = "pgorm_migrations")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub version: String,
    pub applied_at: i64,
    /// The migration's `checksum()` at the time it was applied, or `NULL` for a
    /// row written before the column existed or by a migration that does not
    /// report one.
    pub checksum: Option<String>,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
