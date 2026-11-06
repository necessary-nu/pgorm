use crate::common::migration::*;
use pgorm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220118_000001_create_cake_table::Migration),
            Box::new(m20220118_000002_create_fruit_table::Migration),
            Box::new(m20220118_000003_seed_cake_table::Migration),
            Box::new(m20220118_000004_create_tea_enum::Migration),
            Box::new(m20220923_000001_seed_cake_table::Migration),
        ]
    }

    fn migration_table_name(&self) -> pgorm::Name {
        Name::runtime("override_migration_table_name")
    }
}
