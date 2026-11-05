//! Migrations written as `.sql` files.

use std::fmt::Write;

use pgorm::{ConnectionTrait, DatabaseTransaction, MigrationFile};
use sha2::{Digest, Sha256};

use crate::{Error, MigrationName, MigrationTrait};

/// A migration written as a `.sql` file: named after the file, checksummed by
/// its contents, its grammar checked at compile time.
///
/// The path is relative to the calling crate's manifest directory, the one
/// holding its `Cargo.toml`, so `"src/m20260101_000001_create_note.sql"` names
/// a file beside the crate's Rust migrations. The macro reads the file while
/// the crate compiles and expands to a [`SqlMigration`]:
///
/// - its `name()` is the file stem, as a Rust migration's is with
///   `#[derive(MigrationName)]`;
/// - its `up` runs the file's text through `batch_execute` inside the batch's
///   transaction;
/// - its `checksum()` is the SHA-256 digest of that text, so a file edited
///   after it was applied is reported by every later `status` and `up`.
///
/// ```
/// use pgorm_migration::prelude::*;
///
/// let migration = sql_migration!("tests/sql/m20260101_000001_create_note.sql");
/// assert_eq!(migration.name(), "m20260101_000001_create_note");
/// assert!(migration.sql().contains("CREATE TABLE note ("));
///
/// pub struct Migrator;
///
/// #[async_trait::async_trait]
/// impl MigratorTrait for Migrator {
///     fn migrations() -> Vec<Box<dyn MigrationTrait>> {
///         vec![
///             Box::new(sql_migration!("tests/sql/m20260101_000001_create_note.sql")),
///             Box::new(sql_migration!("tests/sql/m20260101_000002_seed_note.sql")),
///         ]
///     }
/// }
/// ```
///
/// The text is held to PostgreSQL's grammar by libpg_query, as `pgorm::sql!`
/// holds a literal, so a file the server would refuse to parse does not
/// compile. This one differs from the first file above by one keyword:
///
/// ```compile_fail
/// let _ = pgorm_migration::sql_migration!("tests/sql/broken/m20260101_000001_create_note.sql");
/// ```
///
/// The check is syntax only, as `sql!`'s is: a table that does not exist
/// passes, and the server reports it when the migration runs. A psql
/// meta-command such as `\set` is not SQL and is refused.
///
/// The file is an input of the build: editing it recompiles the crate, and the
/// edited text is checked and digested again. Line endings are read as LF, so
/// a checkout that writes CRLF runs the same text and reports the same
/// checksum.
///
/// A file whose first line is [`MigrationFile::COMMITS_AFTER`],
/// `-- pgorm: commits-after`, answers `true` from
/// [`commits_after`](MigrationTrait::commits_after), so the batch commits
/// once it has run. That is how a change PostgreSQL will not let one
/// transaction finish, which [`MigrationFile`] freezes into several files,
/// runs from them:
///
/// ```
/// use pgorm_migration::prelude::*;
///
/// let added = sql_migration!("tests/sql/frozen/m20240401_000002_add_van_1.sql");
/// assert!(added.sql().starts_with("-- pgorm: commits-after\n"));
/// assert!(added.commits_after());
/// assert!(!sql_migration!("tests/sql/m20260101_000001_create_note.sql").commits_after());
/// ```
// [spec:pgorm:def:migration.sql-file]
// [spec:pgorm:req:migration.sql-file.compile+1]    a refusal is a compile error
#[macro_export]
macro_rules! sql_migration {
    ($($input:tt)*) => {
        $crate::__private::sql_migration!($crate, $($input)*)
    };
}

/// A migration whose whole body is one SQL script, which [`sql_migration!`]
/// builds from a `.sql` file.
// [spec:pgorm:def:migration.sql-file]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SqlMigration {
    name: &'static str,
    sql: &'static str,
    commits_after: bool,
}

impl SqlMigration {
    /// A migration named `name` that runs `sql`.
    ///
    /// [`sql_migration!`] is the usual way to build one: it takes both from a
    /// file and checks the text's grammar while the crate compiles. Text given
    /// here is run as it is, unchecked until the server parses it.
    pub const fn new(name: &'static str, sql: &'static str) -> Self {
        Self {
            name,
            sql,
            commits_after: opens_with_commits_after(sql),
        }
    }

    /// The script `up` runs.
    pub const fn sql(&self) -> &'static str {
        self.sql
    }
}

impl MigrationName for SqlMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for SqlMigration {
    // [spec:pgorm:def:migration.sql-file]    the script runs whole, in the batch's transaction
    async fn up(&self, tx: &DatabaseTransaction<'_>) -> Result<(), Error> {
        tx.batch_execute(self.sql).await
    }

    // [spec:pgorm:req:migration.sql-file.commits-after]
    fn commits_after(&self) -> bool {
        self.commits_after
    }

    // [spec:pgorm:req:migration.sql-file.checksum]
    fn checksum(&self) -> Option<String> {
        Some(sha256_hex(self.sql))
    }
}

/// Whether `sql`'s first line is [`MigrationFile::COMMITS_AFTER`], ended by
/// a line break or by the text's end.
// [spec:pgorm:req:migration.sql-file.commits-after]
const fn opens_with_commits_after(sql: &str) -> bool {
    let (text, directive) = (sql.as_bytes(), MigrationFile::COMMITS_AFTER.as_bytes());
    if text.len() < directive.len() {
        return false;
    }
    let mut at = 0;
    while at < directive.len() {
        if text[at] != directive[at] {
            return false;
        }
        at += 1;
    }
    text.len() == directive.len() || text[directive.len()] == b'\n'
}

/// The lowercase hexadecimal SHA-256 digest of `text`'s bytes.
// [spec:pgorm:req:migration.sql-file.checksum]    fixed algorithm, fixed encoding
fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}
