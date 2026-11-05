//! `COPY … FROM STDIN` and `COPY … TO STDOUT` through `pgorm::CopyTrait`.
//!
//! Python chooses a statement's format with a string, so each option is
//! checked against the format, and against the direction, before it reaches
//! the Rust statement that has it as a method.

/// Run `$body` on the statement whatever its format.
macro_rules! every {
    ($kind:ident, $value:expr, |$s:ident| $body:expr) => {
        match $value {
            $kind::Text($s) => {
                $body;
            }
            $kind::Csv($s) => {
                $body;
            }
            $kind::Binary($s) => {
                $body;
            }
        }
    };
}

/// Run `$body` on a text or CSV statement; binary refuses `$option`.
macro_rules! delimited {
    ($kind:ident, $value:expr, $option:literal, |$s:ident| $body:expr) => {
        match $value {
            $kind::Text($s) => {
                $body;
                Ok(())
            }
            $kind::Csv($s) => {
                $body;
                Ok(())
            }
            $kind::Binary(_) => Err(ConstructionError::new_err(concat!(
                $option,
                " is not accepted by the binary format"
            ))),
        }
    };
}

/// Run `$body` on a CSV statement; text and binary refuse `$option`.
macro_rules! csv_only {
    ($kind:ident, $value:expr, $option:literal, |$s:ident| $body:expr) => {
        match $value {
            $kind::Csv($s) => {
                $body;
                Ok(())
            }
            _ => Err(ConstructionError::new_err(concat!(
                $option,
                " requires the csv format"
            ))),
        }
    };
}

mod from;
mod native;
mod pump;
mod to;

pub(crate) use from::CopyFrom;
pub use from::PyCopyFrom;
pub(crate) use pump::{Columns, Message, Piece, Unopened, drive_in, drive_out, open_in, open_out};
pub(crate) use to::CopyTo;
pub use to::PyCopyTo;

use pgorm::pgorm_query::TableName;
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyTuple};
use serde_json::{Map, Value, json};

use crate::{errors::ConstructionError, identifiers::Identifier, statements::PyTable};

/// COPY's data format, as Python names it.
#[derive(Clone, Copy, Debug)]
enum Format {
    Text,
    Csv,
    Binary,
}

fn format(value: &Bound<'_, PyAny>) -> PyResult<Format> {
    match value.extract::<String>().as_deref() {
        Ok("text") => Ok(Format::Text),
        Ok("csv") => Ok(Format::Csv),
        Ok("binary") => Ok(Format::Binary),
        _ => Err(PyTypeError::new_err(
            "COPY format must be \"text\", \"csv\" or \"binary\"",
        )),
    }
}

/// A table COPY names, which takes no alias.
fn table_name(table: &PyTable) -> PyResult<TableName> {
    if table.inner.alias.is_some() {
        return Err(ConstructionError::new_err(
            "COPY takes a table without an alias",
        ));
    }
    Ok(table.inner.name.clone())
}

/// An option's string, which COPY writes as a literal and so cannot hold NUL.
fn text(value: &Bound<'_, PyAny>, option: &str) -> PyResult<String> {
    let text = value
        .extract::<String>()
        .map_err(|_| PyTypeError::new_err(format!("{option} requires a string")))?;
    if text.contains('\0') {
        return Err(ConstructionError::new_err(format!(
            "{option} cannot contain NUL"
        )));
    }
    Ok(text)
}

/// An option that is one character: a one-character string, not NUL.
fn character(value: &Bound<'_, PyAny>, option: &str) -> PyResult<char> {
    let refused =
        || ConstructionError::new_err(format!("{option} requires a one-character string"));
    let text = value
        .extract::<String>()
        .map_err(|_| PyTypeError::new_err(format!("{option} requires a string")))?;
    let mut characters = text.chars();
    match (characters.next(), characters.next()) {
        (Some(character), None) if character != '\0' => Ok(character),
        _ => Err(refused()),
    }
}

fn names(columns: &Bound<'_, PyTuple>) -> PyResult<Vec<Identifier>> {
    columns
        .iter()
        .map(|column| Identifier::new(&column))
        .collect()
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyCopyFrom>()?;
    module.add_class::<PyCopyTo>()?;
    module.add_class::<native::NativeCopyIn>()?;
    module.add_class::<native::NativeCopyOut>()?;
    module.add_function(wrap_pyfunction!(native::copy_in, module)?)?;
    module.add_function(wrap_pyfunction!(native::copy_out, module)?)?;
    Ok(())
}

// [spec:pgorm:req:python.capabilities+2]
// [spec:pgorm:req:bindings.copy]
// [spec:pgorm:req:bindings.copy-rows+1]
pub(crate) fn capabilities() -> Map<String, Value> {
    let builders = [
        (
            "copy_from",
            "pgorm_query::CopyFromStatement::new with CopyText, CopyCsv or CopyBinary",
        ),
        (
            "copy_from.columns",
            "pgorm_query::CopyFromStatement::columns",
        ),
        (
            "copy_from.where",
            "pgorm_query::ConditionalStatement::cond_where for CopyFromStatement",
        ),
        ("copy_from.freeze", "pgorm_query::CopyFromStatement::freeze"),
        (
            "copy_from.delimiter",
            "pgorm_query::CopyFromStatement::delimiter",
        ),
        (
            "copy_from.null_string",
            "pgorm_query::CopyFromStatement::null_string",
        ),
        (
            "copy_from.default_string",
            "pgorm_query::CopyFromStatement::default_string",
        ),
        ("copy_from.header", "pgorm_query::CopyFromStatement::header"),
        (
            "copy_from.header_match",
            "pgorm_query::CopyFromStatement::header_match",
        ),
        (
            "copy_from.encoding",
            "pgorm_query::CopyFromStatement::encoding",
        ),
        (
            "copy_from.on_error_ignore",
            "pgorm_query::CopyFromStatement::on_error_ignore, CopyIgnore, CopyLogVerbosity",
        ),
        ("copy_from.quote", "pgorm_query::CopyFromStatement::quote"),
        ("copy_from.escape", "pgorm_query::CopyFromStatement::escape"),
        (
            "copy_from.force_not_null",
            "pgorm_query::CopyFromStatement::{force_not_null, force_not_null_all}",
        ),
        (
            "copy_from.force_null",
            "pgorm_query::CopyFromStatement::{force_null, force_null_all}",
        ),
        (
            "copy_from.inspect",
            "pgorm_query::CopyFromStatement as Display",
        ),
        (
            "copy_to",
            "pgorm_query::CopyToStatement::new with IntoCopySource or CopySource::try_from, and CopyText, CopyCsv or CopyBinary",
        ),
        (
            "copy_to.delimiter",
            "pgorm_query::CopyToStatement::delimiter",
        ),
        (
            "copy_to.null_string",
            "pgorm_query::CopyToStatement::null_string",
        ),
        ("copy_to.header", "pgorm_query::CopyToStatement::header"),
        ("copy_to.encoding", "pgorm_query::CopyToStatement::encoding"),
        ("copy_to.quote", "pgorm_query::CopyToStatement::quote"),
        ("copy_to.escape", "pgorm_query::CopyToStatement::escape"),
        (
            "copy_to.force_quote",
            "pgorm_query::CopyToStatement::{force_quote, force_quote_all}",
        ),
        ("copy_to.inspect", "pgorm_query::CopyToStatement as Display"),
    ];
    let execution = [
        (
            "connection.copy_in",
            "pgorm::CopyTrait::copy_in for DatabaseConnection, CopyInSink::finish",
        ),
        (
            "connection.copy_out",
            "pgorm::CopyTrait::copy_out for DatabaseConnection, CopyOutStream",
        ),
        (
            "transaction.copy_in",
            "pgorm::CopyTrait::copy_in for DatabaseTransaction, CopyInSink::finish",
        ),
        (
            "transaction.copy_out",
            "pgorm::CopyTrait::copy_out for DatabaseTransaction, CopyOutStream",
        ),
        (
            "connection.copy_in_rows",
            "pgorm::CopyTrait::{copy_in_columns, copy_in} for DatabaseConnection, CopyRowEncoder",
        ),
        (
            "connection.copy_out_rows",
            "pgorm::CopyTrait::{copy_out_columns, copy_out} for DatabaseConnection, CopyRowDecoder",
        ),
        (
            "transaction.copy_in_rows",
            "pgorm::CopyTrait::{copy_in_columns, copy_in} for DatabaseTransaction, CopyRowEncoder",
        ),
        (
            "transaction.copy_out_rows",
            "pgorm::CopyTrait::{copy_out_columns, copy_out} for DatabaseTransaction, CopyRowDecoder",
        ),
    ];
    crate::capabilities::plain(builders.into_iter().chain(execution))
}

pub(crate) fn policy() -> Value {
    json!({
        "formats": ["text", "csv", "binary"],
        "options": "checked against the format and direction at construction; COPY takes no bind parameters, so values render inline",
        "copy_in_sources": ["bytes-like", "iterable of bytes-like", "async iterable of bytes-like"],
        "copy_in_chunking": "small chunks gathered to 64 KiB and large ones sliced to 256 KiB; at most 4 queued",
        "copy_in_abort": "a raising source or a non-bytes item aborts with CopyFail and re-raises; nothing written outside a transaction, the transaction aborted inside one",
        "copy_in_refused": "a COPY FROM the server refuses before data moves closes its connection, and any transaction on it",
        "copy_out_chunks": "bytes joined from received CopyData messages up to 64 KiB; no row boundary is promised",
        "copy_out_buffering": "at most 4 chunks queued; an unread stream holds the server back",
        "ownership": "the copy reserves its connection or transaction until it ends; COPY TO's end releases it, an early close discards it",
        "cancellation": "before finish aborts the COPY; during finish discards the connection or transaction with the outcome unknown",
        "rows": "binary COPY only; the columns are described by the server before the COPY starts, each value read as a parameter is and decoded as a result column is; a row of another width or a value its column's type does not take raises a construction error and aborts the COPY"
    })
}
