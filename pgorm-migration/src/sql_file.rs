//! Migrations written as `.sql` files.

use std::borrow::Cow;
use std::fmt::Write;
use std::io;
use std::path::Path;

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
///     fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
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
// [spec:pgorm:def:migration.sql-file+1]
// [spec:pgorm:req:migration.sql-file.compile+1]    a refusal is a compile error
#[macro_export]
macro_rules! sql_migration {
    ($($input:tt)*) => {
        $crate::__private::sql_migration!($crate, $($input)*)
    };
}

/// A migration whose whole body is one SQL script: what [`sql_migration!`]
/// builds from a `.sql` file while the crate compiles, and what
/// [`SqlMigration::read`] and [`SqlMigration::read_dir`] read from files as
/// the program runs.
// [spec:pgorm:def:migration.sql-file+1]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlMigration {
    name: Cow<'static, str>,
    sql: Cow<'static, str>,
    commits_after: bool,
}

impl SqlMigration {
    /// A migration named `name` that runs `sql`.
    ///
    /// [`sql_migration!`] is the usual way to build one: it takes both from a
    /// file and checks the text's grammar while the crate compiles. Text given
    /// here is run as it is, unchecked until the server parses it.
    pub fn new(name: impl Into<Cow<'static, str>>, sql: impl Into<Cow<'static, str>>) -> Self {
        let sql = sql.into();
        Self {
            name: name.into(),
            commits_after: opens_with_commits_after(&sql),
            sql,
        }
    }

    /// The migration the `.sql` file at `path` holds, read as the program
    /// runs: named for the file's stem, as [`sql_migration!`] names one, its
    /// text the file's with every CRLF line ending read as LF, so it reports
    /// the checksum the macro would.
    ///
    /// The text is held to PostgreSQL's grammar as `sql_migration!` holds a
    /// file, syntax only, so a file the server would refuse to parse is
    /// refused here rather than when its batch runs.
    ///
    /// A path naming no file stem is [`io::ErrorKind::InvalidInput`], and
    /// text that is not UTF-8, or that the grammar refuses,
    /// [`io::ErrorKind::InvalidData`], the grammar's refusal naming the file
    /// and carrying the parser's diagnostic.
    // [spec:pgorm:req:migration.sql-file.read]
    pub fn read(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{} names no file stem", path.display()),
                )
            })?
            .to_owned();
        let text = String::from_utf8(std::fs::read(path)?).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not UTF-8 text", path.display()),
            )
        })?;
        let text = text.replace("\r\n", "\n");
        if let Err(refused) = pgorm_sql_check::check_sql(&text) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {refused}", path.display()),
            ));
        }
        Ok(Self::new(name, text))
    }

    /// Every `.sql` file directly inside `directory`, each read as
    /// [`read`](SqlMigration::read) reads one, in the order of their file
    /// names: the order a directory of migrations named
    /// `m<timestamp>_<description>.sql`, as [`MigrationFile`] freezes them,
    /// was written in.
    ///
    /// A file name that is not UTF-8 is [`io::ErrorKind::InvalidData`];
    /// subdirectories and files with another extension are passed over.
    // [spec:pgorm:req:migration.sql-file.read]
    pub fn read_dir(directory: impl AsRef<Path>) -> io::Result<Vec<Self>> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file()
                || path.extension().and_then(|extension| extension.to_str()) != Some("sql")
            {
                continue;
            }
            let file_name = entry.file_name().into_string().map_err(|name| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} is not a UTF-8 file name", name.display()),
                )
            })?;
            files.push((file_name, path));
        }
        files.sort();
        files
            .into_iter()
            .map(|(_, path)| Self::read(path))
            .collect()
    }

    /// The script `up` runs.
    pub fn sql(&self) -> &str {
        &self.sql
    }
}

impl MigrationName for SqlMigration {
    fn name(&self) -> &str {
        &self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for SqlMigration {
    // [spec:pgorm:def:migration.sql-file+1]    the script runs whole, in the batch's transaction
    async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        tx.batch_execute(&self.sql).await
    }

    // [spec:pgorm:req:migration.sql-file.commits-after]
    fn commits_after(&self) -> bool {
        self.commits_after
    }

    // [spec:pgorm:req:migration.sql-file.checksum]
    fn checksum(&self) -> Option<String> {
        Some(sha256_hex(&self.sql))
    }
}

/// Whether `sql`'s first line is [`MigrationFile::COMMITS_AFTER`], ended by
/// a line break or by the text's end.
// [spec:pgorm:req:migration.sql-file.commits-after]
fn opens_with_commits_after(sql: &str) -> bool {
    sql.strip_prefix(MigrationFile::COMMITS_AFTER)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('\n'))
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
