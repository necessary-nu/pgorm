//! Query terminals use pgorm's cached executor, ValueHolder, and Rust FromSql.

mod codecs;
mod decode;
mod execute;
mod json;
mod ranges;
mod record;
mod stream;
#[cfg(test)]
mod tests;

pub(crate) use decode::{Field, value};
pub(crate) use execute::{execute, fetch};
pub use record::{PyField, PyRecord};
pub(crate) use stream::{RowDecode, Source, open};

use pyo3::prelude::*;

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyField>()?;
    module.add_class::<PyRecord>()?;
    module.add_class::<stream::NativeStream>()?;
    Ok(())
}

pub(crate) fn capabilities() -> serde_json::Map<String, serde_json::Value> {
    crate::capabilities::plain([
        ("connection.execute", "pgorm::ConnectionTrait::execute"),
        ("connection.query", "pgorm::ConnectionTrait::query_all"),
        ("connection.one", "pgorm::ConnectionTrait::query_all"),
        ("connection.optional", "pgorm::ConnectionTrait::query_all"),
        ("connection.stream", "pgorm::ConnectionTrait::query_raw"),
        (
            "transaction.stream",
            "pgorm::ConnectionTrait::query_raw for DatabaseTransaction",
        ),
        (
            "connection.batch_execute",
            "pgorm::ConnectionTrait::batch_execute",
        ),
        ("connection.verify", "pgorm::VerifyStatement::describe"),
        ("connection.discard", "pgorm::DatabaseConnection::discard"),
        (
            "connection.transaction_with_retry",
            "pgorm::DatabaseConnection::transaction_with_retry",
        ),
        ("sql", "pgorm::sql_check::check_sql"),
        ("prql", "pgorm::sql_check::compile_prql"),
        ("from_statement", "pgorm::SelectorRaw::from_statement"),
        (
            "selector_raw.paginate",
            "pgorm::raw_select, pgorm::page_statement",
        ),
        (
            "selector_raw.count",
            "pgorm::raw_select, pgorm::count_statement",
        ),
        (
            "model.find_by_statement",
            "pgorm::FromQueryResult::find_by_statement",
        ),
        ("entity_query.from_raw_sql", "pgorm::Select::from_raw_sql"),
        ("record", "tokio_postgres::Row::try_get / FromSql"),
    ])
}
