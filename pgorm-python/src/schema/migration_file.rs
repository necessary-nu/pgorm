//! Generated DDL frozen as a migration written as a `.sql` file: pgorm's
//! [`MigrationFile`], whose text and whose freezing are pgorm's alone.

use std::path::PathBuf;

use pgorm::MigrationFile;
use pyo3::prelude::*;

use super::statement::PyDDL;
use crate::errors::ConstructionError;

/// Statements frozen as the text of a migration written as a `.sql` file,
/// written once by `freeze` and never again.
// [spec:pgorm:req:bindings.migration-file+1]
#[derive(Debug)]
#[pyclass(name = "MigrationFile", module = "pgorm.schema", frozen)]
pub struct PyMigrationFile {
    inner: MigrationFile,
}

impl From<MigrationFile> for PyMigrationFile {
    fn from(inner: MigrationFile) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyMigrationFile {
    /// The file's stem, and the migration's name.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// The file's text, which the migration runs whole.
    #[getter]
    fn text(&self) -> &str {
        self.inner.text()
    }

    /// Whether the text opens with `-- pgorm: commits-after`.
    #[getter]
    fn commits_after(&self) -> bool {
        self.inner.commits_after()
    }

    /// Write `<name>.sql` into `directory` if no file of that name is there,
    /// answering whether it wrote one: a file already there is never
    /// rewritten. A name that is no plain file stem is a `ConstructionError`,
    /// and any other failure the `OSError` it is.
    // [spec:pgorm:req:bindings.migration-file+1]
    fn freeze(&self, directory: PathBuf) -> PyResult<bool> {
        self.inner
            .freeze(directory)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::InvalidInput => ConstructionError::new_err(error.to_string()),
                _ => error.into(),
            })
    }

    fn __repr__(&self) -> String {
        format!("MigrationFile({:?})", self.inner.name())
    }

    // [spec:pgorm:req:python.truth-testing+5]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "a migration file cannot be tested as a Python boolean",
        ))
    }
}

/// `statements` frozen as the migration `name`, each ending `;` on a line of
/// its own, opening with `-- pgorm: commits-after` where `commits_after`.
// [spec:pgorm:req:bindings.migration-file+1]
#[pyfunction]
#[pyo3(signature = (name, statements, *, commits_after = false))]
pub(super) fn migration_file(
    name: String,
    statements: Vec<PyRef<'_, PyDDL>>,
    commits_after: bool,
) -> PyMigrationFile {
    let file = MigrationFile::new(
        name,
        statements.iter().map(|statement| statement.inner.sql()),
    );
    match commits_after {
        true => file.committing_after(),
        false => file,
    }
    .into()
}
