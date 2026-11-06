use crate::common::migration::m20220118_000001_create_cake_table;
use pgorm_migration::prelude::*;

/// The digest of `tests/sql/m20260101_000001_create_note.sql`, as
/// `shasum -a 256` reports it for the file's LF text.
pub const CREATE_NOTE_DIGEST: &str =
    "9ae0994f5653f1500a920aee8881629d9aafeead971627b2f54bc0ace1b1bb17";

/// The digest of the same file after the edit in `tests/sql/edited/`.
pub const EDITED_NOTE_DIGEST: &str =
    "e6dfb83b76e567fe17349e94f5ef3cbd5b25d06ac9cc11ce36ca402eb0f87891";

/// A migration written in Rust, then two written as SQL files.
pub struct Written;

#[async_trait::async_trait]
impl MigratorTrait for Written {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220118_000001_create_cake_table::Migration),
            Box::new(sql_migration!("tests/sql/m20260101_000001_create_note.sql")),
            Box::new(sql_migration!("tests/sql/m20260101_000002_seed_note.sql")),
        ]
    }
}

/// [`Written`], its first SQL file edited after it was applied: same file
/// name, different text.
pub struct Edited;

#[async_trait::async_trait]
impl MigratorTrait for Edited {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220118_000001_create_cake_table::Migration),
            Box::new(sql_migration!(
                "tests/sql/edited/m20260101_000001_create_note.sql"
            )),
            Box::new(sql_migration!("tests/sql/m20260101_000002_seed_note.sql")),
        ]
    }
}
