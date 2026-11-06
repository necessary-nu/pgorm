pub use pgorm_migration::prelude::*;

mod m20220101_000001_create_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // A migration written in Rust, with the query builders.
            Box::new(m20220101_000001_create_table::Migration),
            // A migration written as a SQL file, named after the file. The
            // path is relative to this crate's Cargo.toml.
            Box::new(sql_migration!("src/m20220101_000002_index_post_title.sql")),
        ]
    }
}
