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
pub(crate) use execute::{execute, fetch, fetch_tuples};
pub use record::{PyField, PyRecord};
pub(crate) use stream::{RowDecode, open};

use pyo3::prelude::*;

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyField>()?;
    module.add_class::<PyRecord>()?;
    module.add_class::<stream::NativeStream>()?;
    Ok(())
}

pub(crate) fn capabilities() -> serde_json::Map<String, serde_json::Value> {
    [
        ("connection.execute", "pgorm::ConnectionTrait::execute"),
        ("connection.query", "pgorm::ConnectionTrait::query_all"),
        ("connection.one", "pgorm::ConnectionTrait::query_all"),
        ("connection.optional", "pgorm::ConnectionTrait::query_all"),
        ("connection.stream", "pgorm::ConnectionTrait::query_raw"),
        ("record", "tokio_postgres::Row::try_get / FromSql"),
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
