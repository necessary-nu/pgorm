//! Generated DDL frozen as a migration written as a `.sql` file, so a
//! history of migrations runs as it was written on a database that has none
//! of it, whatever the declarations it was generated from say by then.

use super::{DisjointChange, DisjointStep};
use std::{
    fmt,
    fs::OpenOptions,
    io::{self, Write},
    path::Path,
};

/// Statements a program generates, as the text of a migration written as a
/// `.sql` file, which pgorm-migration's `sql_migration!` runs.
///
/// DDL generated from a declaration, by
/// [`create_table_from_entity`](crate::Schema::create_table_from_entity) or a
/// disjoint change, describes the declaration as it stands when it is
/// generated. A migration that generated it each time it ran would follow the
/// declaration as it changed, and a database built from the history would get
/// today's tables from the first migration and then the later migrations'
/// changes to them again. So the DDL is generated once, when the change is
/// made, and [`freeze`](Self::freeze) writes it to a file the migrations
/// list, which nothing rewrites.
///
/// ```
/// use pgorm::MigrationFile;
/// use pgorm::pgorm_query::{ColumnDef, Name, Table};
///
/// let table = Table::create(Name::runtime("note"))
///     .col(ColumnDef::new(Name::runtime("id")).integer().not_null())
///     .to_owned();
/// let file = MigrationFile::new("m20260101_000001_create_note", [table]);
/// assert_eq!(
///     file.text(),
///     "CREATE TABLE \"note\" ( \"id\" integer NOT NULL );\n"
/// );
/// ```
// [spec:pgorm:def:migration.frozen]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationFile {
    name: String,
    text: String,
}

impl MigrationFile {
    /// The line a migration file opens with to commit, with its ledger row,
    /// before the next migration of the batch runs: pgorm-migration's
    /// `SqlMigration` reads it as its `commits_after()`.
    // [spec:pgorm:req:migration.sql-file.commits-after]
    pub const COMMITS_AFTER: &str = "-- pgorm: commits-after";

    /// The migration `name` running `statements` in order, each ending `;` on
    /// a line of its own. `name` is the file's stem, and the migration's name
    /// in the ledger.
    // [spec:pgorm:def:migration.frozen]
    pub fn new<S: fmt::Display>(
        name: impl Into<String>,
        statements: impl IntoIterator<Item = S>,
    ) -> Self {
        let text = statements
            .into_iter()
            .map(|statement| format!("{statement};\n"))
            .collect();
        Self {
            name: name.into(),
            text,
        }
    }

    /// The same migration, opening with [`COMMITS_AFTER`](Self::COMMITS_AFTER) so that it commits
    /// before the next migration of its batch runs, as an
    /// `ALTER TYPE .. ADD VALUE` whose value a later migration uses must.
    // [spec:pgorm:req:migration.sql-file.commits-after]
    pub fn committing_after(mut self) -> Self {
        if !self.commits_after() {
            self.text = format!("{}\n{}", Self::COMMITS_AFTER, self.text);
        }
        self
    }

    /// Whether the migration commits before the next one runs.
    // [spec:pgorm:req:migration.sql-file.commits-after]
    pub fn commits_after(&self) -> bool {
        self.text
            .strip_prefix(Self::COMMITS_AFTER)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('\n'))
    }

    /// The file's stem, and the migration's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The file's text, which the migration runs whole.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Write the file as `<name>.sql` in `dir` if no file of that name is
    /// there, answering whether it wrote one.
    ///
    /// A file already there is left as it is: it is the migration as it was
    /// written, which databases may have run and recorded the checksum of, so
    /// the code that froze a change can keep running after the declaration
    /// it was generated from has moved on. A name that is not a plain file
    /// stem — empty, `.` or `..`, or holding a path separator or a NUL — is
    /// refused as [`io::ErrorKind::InvalidInput`], as is a `dir` that does
    /// not exist as [`io::ErrorKind::NotFound`].
    // [spec:pgorm:def:migration.frozen]
    pub fn freeze(&self, dir: impl AsRef<Path>) -> io::Result<bool> {
        if self.name.is_empty()
            || self.name == "."
            || self.name == ".."
            || self.name.contains(['/', '\\', '\0'])
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("`{}` is not a migration file's stem", self.name),
            ));
        }
        let path = dir.as_ref().join(format!("{}.sql", self.name));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(self.text.as_bytes())?;
                Ok(true)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error),
        }
    }
}

impl DisjointChange {
    /// The change frozen as migration files, one per step in order: named
    /// `name` for a change of one step and `<name>_<n>` from 1 otherwise,
    /// every step but the last [committing after](MigrationFile::committing_after),
    /// and a step after which the program writes rows ending in a comment
    /// saying which, which a migration of the program's own writes before the
    /// next file runs.
    // [spec:pgorm:sem:disjoint.evolve.frozen]
    pub fn migration_files(&self, name: &str) -> Vec<MigrationFile> {
        let steps = self.steps();
        steps
            .iter()
            .enumerate()
            .map(|(index, step)| {
                let name = match steps.len() {
                    1 => name.to_owned(),
                    _ => format!("{name}_{}", index + 1),
                };
                let file = step_file(name, step);
                match index + 1 < steps.len() {
                    true => file.committing_after(),
                    false => file,
                }
            })
            .collect()
    }
}

/// One step as a file: its statements, then the rows it leaves the program
/// to write, if any.
// [spec:pgorm:sem:disjoint.evolve.frozen]
fn step_file(name: String, step: &DisjointStep) -> MigrationFile {
    let mut file = MigrationFile::new(name, step.statements());
    if let Some(gap) = step.gap() {
        file.text.push_str(&format!(
            "-- Before the next migration runs, write {gap}.\n"
        ));
    }
    file
}
