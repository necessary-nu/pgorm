//! What raw SQL does beside its terminals: a script run unprepared, SQL text
//! held to the grammar, PRQL text compiled, a statement described without
//! running it, and a `SELECT` wrapped for its pages and its count. Each is
//! pgorm's own: `batch_execute`, `pgorm::sql_check`, `VerifyStatement::describe`
//! and `raw_select`.

use pgorm::{CopyColumn, Error, VerifyStatement, sql_check};
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyString};
use pyo3_async_runtimes::tokio::future_into_py;

use crate::{
    errors::{ConstructionError, InternalError},
    execution::{Database, Executor},
    results::{self, Field},
    statements::{PySelect, RawSQL},
    transactions::work::{Output, Work},
    values::PyValue,
};

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(sql, module)?)?;
    module.add_function(wrap_pyfunction!(prql, module)?)?;
    module.add_function(wrap_pyfunction!(raw_select, module)?)?;
    module.add_function(wrap_pyfunction!(batch_execute, module)?)?;
    module.add_function(wrap_pyfunction!(describe, module)?)?;
    Ok(())
}

/// `text`, held to PostgreSQL 18's grammar and returned unchanged, as Rust's
/// `sql!` holds a literal while its crate compiles. A script is held to it
/// statement by statement. What the grammar rejects raises
/// `ConstructionError` with the parser's message and a caret under the token
/// it stopped at.
// [spec:pgorm:req:bindings.sql-check]
#[pyfunction]
fn sql<'py>(text: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let checked = text
        .cast::<PyString>()
        .map_err(|_| PyTypeError::new_err("SQL text is a str"))?
        .to_str()?;
    sql_check::check_sql(checked)
        .map_err(|rejection| ConstructionError::new_err(rejection.to_string()))?;
    Ok(text.clone())
}

/// `text`, PRQL, compiled to PostgreSQL SQL and held to the grammar, as a
/// `RawSQL` binding `values` to its placeholders in order, as Rust's `prql!`
/// expands. One value per distinct placeholder.
// [spec:pgorm:req:bindings.prql-text]
#[pyfunction]
#[pyo3(signature = (text, values=None))]
fn prql(text: &Bound<'_, PyAny>, values: Option<&Bound<'_, PyAny>>) -> PyResult<RawSQL> {
    let text = text
        .cast::<PyString>()
        .map_err(|_| PyTypeError::new_err("PRQL text is a str"))?
        .to_str()?;
    let compiled = sql_check::compile_prql(text)
        .map_err(|rejection| ConstructionError::new_err(rejection.to_string()))?;
    let statement = RawSQL::new(compiled.sql().to_owned(), values)?;
    compiled
        .check_arity(statement.values.0.len())
        .map_err(|rejection| ConstructionError::new_err(rejection.to_string()))?;
    Ok(statement)
}

/// `statement` as `SELECT * FROM (statement) AS "sub_statement"`, the select
/// a raw statement's pages and count are read through, refused unless the
/// statement is one row-returning `SELECT`.
// [spec:pgorm:req:bindings.raw-pages]
#[pyfunction(name = "_raw_select")]
fn raw_select(statement: PyRef<'_, RawSQL>) -> PyResult<PySelect> {
    pgorm::raw_select(&statement.template, statement.values.0.clone())
        .map(|inner| PySelect { inner })
        .map_err(|error| match error {
            Error::Query(refusal) => ConstructionError::new_err(refusal.to_string()),
            other => ConstructionError::new_err(other.to_string()),
        })
}

/// Run `script`, `;`-separated statements with no parameters, through the
/// simple-query protocol, preparing nothing.
// [spec:pgorm:req:bindings.batch-execute]
#[pyfunction(name = "_batch_execute")]
fn batch_execute<'py>(
    py: Python<'py>,
    db: &Bound<'_, PyAny>,
    script: &Bound<'_, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let script = script
        .cast::<PyString>()
        .map_err(|_| PyTypeError::new_err("a script is a str"))?
        .to_str()?
        .to_owned();
    if script.contains('\0') {
        return Err(ConstructionError::new_err("SQL text cannot contain NUL"));
    }
    let executor = Executor::extract(py, db)?;
    future_into_py(py, async move {
        match executor.run(Work::Batch(script)).await? {
            Output::Finished => Ok(()),
            _ => Err(InternalError::new_err("unexpected script result")),
        }
    })
}

/// The result columns the server describes for `statement`, without running
/// it: each column's name, and a `NULL` of the kind its rows decode as.
// [spec:pgorm:req:bindings.verify]
#[pyfunction(name = "_describe")]
fn describe<'py>(
    py: Python<'py>,
    db: &Bound<'_, PyAny>,
    statement: PyRef<'_, RawSQL>,
) -> PyResult<Bound<'py, PyAny>> {
    let sql = statement.template.clone();
    let executor = Executor::extract(py, db)?;
    future_into_py(py, async move {
        let Output::Described(columns) = executor.run(Work::Describe(sql)).await? else {
            return Err(InternalError::new_err("unexpected description"));
        };
        // A type the binding reads no value of has no kind, which a
        // declared field's kind never matches.
        Ok(columns
            .into_iter()
            .enumerate()
            .map(|(index, column)| {
                let copy = CopyColumn::new(column.name(), column.type_().clone());
                let kind = results::value(Field::Copy {
                    column: &copy,
                    raw: None,
                    index,
                })
                .ok();
                (
                    column.name().to_owned(),
                    column.type_().name().to_owned(),
                    kind,
                )
            })
            .collect::<Vec<(String, String, Option<PyValue>)>>())
    })
}

/// `sql`'s description on `db`.
pub(crate) async fn described(
    db: Database<'_>,
    sql: &str,
) -> Result<Vec<pgorm::DescribedColumn>, Error> {
    match db {
        Database::Connection(connection) => connection.describe(sql).await,
        Database::Transaction(transaction) => transaction.describe(sql).await,
        Database::MeteredConnection(connection) => connection.inner().describe(sql).await,
        Database::MeteredTransaction(transaction) => transaction.inner().describe(sql).await,
    }
}
