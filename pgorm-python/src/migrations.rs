//! pgorm-migration's runner, run from Python.
//!
//! A Python program's migrations are pgorm-bindings' list of them, whose
//! written migrations' `up` awaits the Python object's `up` in the batch's
//! transaction.

use std::{fmt, path::PathBuf, sync::Arc};

use pgorm_bindings::migrations::{Entry, Failed, Host, Listed};
use pgorm_migration::{MigrationName, MigrationTrait, SqlMigration};
use pyo3::{exceptions::PyTypeError, intern, prelude::*, types::PyString};
use pyo3_async_runtimes::TaskLocals;

use crate::{errors::Raise, runtime::Owned};
use crate::{
    errors::{ConstructionError, InternalError},
    execution::Executor,
    panics::future_into_py,
    runtime::{Loop, NativePool, PoolState},
    transactions::{
        NativeTransaction,
        work::{Ledger, Output, Work},
    },
};

/// A migration whose whole body is one SQL script, pgorm-migration's
/// `SqlMigration`: what a `.sql` file holds, read as the program runs.
// [spec:pgorm:req:bindings.migrations]
#[derive(Debug)]
#[pyclass(name = "SqlMigration", module = "pgorm.schema", frozen)]
pub(crate) struct PySqlMigration {
    inner: SqlMigration,
}

/// An I/O failure as Python raises it: a path naming no file, text that is
/// not UTF-8 and text the grammar refuses a `ConstructionError`, as a
/// migration file's name that is no file stem is, and anything else the
/// `OSError` it is.
fn io_error(error: std::io::Error) -> PyErr {
    match error.kind() {
        std::io::ErrorKind::InvalidInput | std::io::ErrorKind::InvalidData => {
            ConstructionError::new_err(error.to_string())
        }
        _ => error.into(),
    }
}

#[pymethods]
impl PySqlMigration {
    /// The migration `name` running `sql`.
    #[new]
    fn new(name: String, sql: String) -> Self {
        Self {
            inner: SqlMigration::new(name, sql),
        }
    }

    /// The migration the `.sql` file at `path` holds, named for its stem.
    #[staticmethod]
    fn read(path: PathBuf) -> PyResult<Self> {
        SqlMigration::read(path)
            .map(|inner| Self { inner })
            .map_err(io_error)
    }

    /// Every `.sql` file directly inside `directory`, in file-name order.
    #[staticmethod]
    fn read_dir(directory: PathBuf) -> PyResult<Vec<Self>> {
        SqlMigration::read_dir(directory)
            .map(|files| files.into_iter().map(|inner| Self { inner }).collect())
            .map_err(io_error)
    }

    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    #[getter]
    fn sql(&self) -> &str {
        self.inner.sql()
    }

    /// The SHA-256 digest of the text, which the ledger records.
    #[getter]
    fn checksum(&self) -> Option<String> {
        self.inner.checksum()
    }

    #[getter]
    fn commits_after(&self) -> bool {
        self.inner.commits_after()
    }

    fn __repr__(&self) -> String {
        format!("SqlMigration({:?})", self.inner.name())
    }

    // [spec:pgorm:req:python.truth-testing+5]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "a migration cannot be tested as a Python boolean",
        ))
    }
}

/// A migration written in Python: the object whose `up` is awaited.
type Written = pgorm_bindings::migrations::Written<Py<PyAny>>;

/// What the runner reads of a migration written in Python, once, as the
/// list is built.
// [spec:pgorm:req:bindings.migrations]    name, checksum and commits_after read once
fn written(object: &Bound<'_, PyAny>) -> PyResult<Written> {
    let py = object.py();
    let name = object
        .getattr(intern!(py, "name"))
        .ok()
        .and_then(|name| name.extract::<String>().ok())
        .ok_or_else(|| PyTypeError::new_err("a migration has a str name"))?;
    if !object
        .getattr(intern!(py, "up"))
        .is_ok_and(|up| up.is_callable())
    {
        return Err(PyTypeError::new_err(format!(
            "migration {name:?} has no up method"
        )));
    }
    let checksum = match object.getattr_opt(intern!(py, "checksum"))? {
        Some(checksum) if !checksum.is_none() => Some(
            checksum
                .cast::<PyString>()
                .map_err(|_| {
                    PyTypeError::new_err(format!("migration {name:?}'s checksum is a str"))
                })?
                .to_string(),
        ),
        _ => None,
    };
    let commits_after = match object.getattr_opt(intern!(py, "commits_after"))? {
        Some(commits_after) => commits_after.extract::<bool>().map_err(|_| {
            PyTypeError::new_err(format!("migration {name:?}'s commits_after is a bool"))
        })?,
        None => false,
    };
    Ok(Written {
        name,
        checksum,
        commits_after,
        object: object.clone().unbind(),
    })
}

/// What one `up` call's Python migrations run with: the pool their
/// transaction handles report through, and the event loop and context the
/// call was made in.
struct Call {
    pool: Arc<PoolState>,
    locals: TaskLocals,
}

impl Host for Call {
    type Object = Py<PyAny>;
    type Owner = Loop;
    type Raised = PyErr;

    fn pool(&self) -> &Arc<PoolState> {
        &self.pool
    }

    async fn up(
        self: &Arc<Self>,
        written: &Arc<Written>,
        state: Arc<pgorm_bindings::transaction::State<Loop>>,
    ) -> Result<(), Failed<PyErr>> {
        let running = Python::attach(|py| {
            let native = NativeTransaction { state };
            let awaitable = py
                .import(intern!(py, "pgorm.schema.migrations"))?
                .getattr(intern!(py, "_run_up"))?
                .call1((written.object.clone_ref(py), native))?;
            pyo3_async_runtimes::into_future_with_locals(&self.locals, awaitable)
        });
        match running {
            Ok(running) => running.await.map(drop),
            Err(raised) => Err(raised),
        }
        .map_err(|raised| Failed::With(raised, "raised"))
    }
}

/// A migrator built from Python's list: pgorm-migration's `Migrations`, its
/// ledger placed and its channel named as given.
// [spec:pgorm:req:bindings.migrations]
#[pyclass(name = "_Migrations", module = "pgorm._native", frozen)]
pub(crate) struct NativeMigrations(Listed<Py<PyAny>>);

impl fmt::Debug for NativeMigrations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeMigrations")
            .field("migrations", &self.0.len())
            .finish_non_exhaustive()
    }
}

impl NativeMigrations {
    fn ledger<'py>(
        &self,
        py: Python<'py>,
        executor: &Bound<'_, PyAny>,
        ledger: Ledger,
    ) -> PyResult<Bound<'py, PyAny>> {
        let executor = Executor::extract(py, executor)?;
        let work = Work::Ledger(Box::new(self.0.ledger::<Call>()), ledger);
        future_into_py(py, async move {
            match executor.run(work).await? {
                Output::Migrations(migrations) => Ok(Some(migrations)),
                Output::Finished => Ok(None),
                _ => Err(InternalError::new_err("unexpected migration result")),
            }
        })
    }
}

#[pymethods]
impl NativeMigrations {
    #[new]
    #[pyo3(signature = (migrations, migration_table_name, migration_table_schema, statement_cache_channel))]
    fn new(
        migrations: Vec<Bound<'_, PyAny>>,
        migration_table_name: Option<String>,
        migration_table_schema: Option<String>,
        statement_cache_channel: Option<String>,
    ) -> PyResult<Self> {
        let entries = migrations
            .iter()
            .map(|migration| match migration.cast::<PySqlMigration>() {
                Ok(sql) => Ok(Entry::Sql(sql.get().inner.clone())),
                Err(_) => written(migration).map(|written| Entry::Written(Arc::new(written))),
            })
            .collect::<PyResult<_>>()?;
        Ok(Self(Listed::new(
            entries,
            migration_table_name,
            migration_table_schema,
            statement_cache_channel,
        )))
    }

    /// Apply the pending migrations through pgorm-migration's `up`, on
    /// connections of `pool`'s own.
    // [spec:pgorm:req:bindings.migrations]
    #[pyo3(signature = (pool, steps))]
    fn up<'py>(
        &self,
        py: Python<'py>,
        pool: &Bound<'_, PyAny>,
        steps: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let native = pool.getattr("_native").unwrap_or_else(|_| pool.clone());
        let pool = native
            .cast::<NativePool>()
            .map_err(|_| PyTypeError::new_err("migrations run on a Pool"))?
            .get()
            .state
            .clone();
        pool.check_owner(py)?;
        pool.ensure_open().py()?;
        let call = Call {
            pool,
            locals: TaskLocals::with_running_loop(py)?.copy_context(py)?,
        };
        let up = self.0.up(call, steps);
        future_into_py(py, async move {
            match up.await.py()? {
                None => Ok(()),
                Some(raised) => Err(raised),
            }
        })
    }

    /// Create the ledger, and its schema, where they are missing.
    fn install<'py>(
        &self,
        py: Python<'py>,
        executor: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.ledger(py, executor, Ledger::Install)
    }

    /// Each migration's name and whether the ledger records it applied.
    fn status<'py>(
        &self,
        py: Python<'py>,
        executor: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.ledger(py, executor, Ledger::Status)
    }

    fn __repr__(&self) -> String {
        format!("<pgorm native migrations of {}>", self.0.len())
    }
}

/// The runner's operations, each the pgorm-migration API it calls.
// [spec:pgorm:req:python.capabilities+2]
pub(crate) fn capabilities() -> serde_json::Map<String, serde_json::Value> {
    [
        (
            "migrations.up",
            "pgorm_migration::MigratorTrait::up for Migrations",
        ),
        (
            "migrations.install",
            "pgorm_migration::MigratorTrait::install",
        ),
        (
            "migrations.status",
            "pgorm_migration::MigratorTrait::get_migration_with_status",
        ),
        ("sql_migration", "pgorm_migration::SqlMigration::new"),
        ("sql_migration.read", "pgorm_migration::SqlMigration::read"),
        (
            "sql_migration.read_dir",
            "pgorm_migration::SqlMigration::read_dir",
        ),
    ]
    .into_iter()
    .map(|(name, api)| {
        (
            name.to_owned(),
            serde_json::json!({"rust_api": api, "features": []}),
        )
    })
    .collect()
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PySqlMigration>()?;
    module.add_class::<NativeMigrations>()?;
    Ok(())
}
