//! pgorm-query's DDL builders, from Python: schemas, tables and their
//! columns, keys and constraints, their alterations, indexes, types,
//! domains, sequences, views, materialized views and tables created from a
//! query, extensions and comments. Construction never performs database
//! work.
//!
//! The typestates pgorm-query holds — an `ALTER TABLE`, `ALTER TYPE`,
//! `ALTER DOMAIN` or `ALTER SEQUENCE` before its first action — are classes
//! of their own, which are no `DDL`: no terminal runs one, and none has an
//! `inspect()`.

mod alter;
mod args;
mod capabilities;
mod column;
mod custom_types;
mod domain;
mod drops;
mod entity;
mod index;
mod keys;
mod migration_file;
mod namespace;
mod sequence;
mod statement;
mod table;
mod types;
mod view;

pub(crate) use capabilities::operations as capabilities;
pub use column::PyColumnDef;
pub use custom_types::PyCreateType;
pub use entity::PyEntitySchema;
pub use index::PyCreateIndex;
pub use migration_file::PyMigrationFile;
use pyo3::prelude::*;
pub use statement::PyDDL;
pub use table::PyCreateTable;
pub use types::PyDataType;

pub(crate) use args::relation;

/// `statement`, one of a list pgorm's `Schema` built, as Python's class of
/// its kind: `CreateType`, `CreateTable` and `AlterTable` for the builders
/// that take more, and `DDL` for the rest.
// [spec:pgorm:req:bindings.disjoint+2]
// [spec:pgorm:req:bindings.disjoint-evolve]
pub(crate) fn ddl_object(
    py: Python<'_>,
    statement: pgorm::DdlStatement,
) -> PyResult<Bound<'_, PyAny>> {
    use pgorm::DdlStatement;
    let plain = |statement| Ok(Bound::new(py, PyDDL::new(statement))?.into_any());
    match statement {
        DdlStatement::CreateType(statement) => Ok(PyCreateType::object(py, statement)?.into_any()),
        DdlStatement::CreateTable(statement) => {
            Ok(PyCreateTable::object(py, statement)?.into_any())
        }
        DdlStatement::AlterTable(statement) => {
            let table = statement.get_table_name().clone();
            Ok(
                alter::PyAlterTable::object(py, statement::AlteredTable { table, statement })?
                    .into_any(),
            )
        }
        DdlStatement::AlterType(statement) => plain(statement::Statement::AlterType(statement)),
        DdlStatement::DropType(statement) => plain(statement::Statement::DropType(statement)),
        DdlStatement::DropTable(statement) => plain(statement::Statement::DropTable(statement)),
        _ => Err(crate::errors::InternalError::new_err(
            "a schema statement of a kind this build does not know",
        )),
    }
}

/// Why `value`, a schema part that is no statement, cannot be inspected or
/// run: PostgreSQL parses no `ALTER` without an action, and a column is no
/// statement. `None` for anything else.
// [spec:pgorm:req:bindings.schema]
pub(crate) fn refusal(value: &Bound<'_, PyAny>) -> Option<&'static str> {
    if value.is_instance_of::<column::PyColumnDef>() {
        Some("a ColumnDef is not a statement to run: add it to a table")
    } else if value.is_instance_of::<alter::PyPendingAlterTable>() {
        Some(
            "an ALTER TABLE needs an action before it can be inspected or run: add_column(..), \
             drop_column(..), add_check(..) and the rest",
        )
    } else if value.is_instance_of::<custom_types::PyPendingAlterType>() {
        Some(
            "an ALTER TYPE needs a change before it can be inspected or run: add_value(..), \
             rename_to(..), add_attribute(..) and the rest",
        )
    } else if value.is_instance_of::<domain::PyPendingAlterDomain>() {
        Some(
            "an ALTER DOMAIN needs an action before it can be inspected or run: set_default(..), \
             add_check(..), rename_to(..) and the rest",
        )
    } else if value.is_instance_of::<sequence::PyPendingAlterSequence>() {
        Some(
            "an ALTER SEQUENCE needs a clause before it can be inspected or run: options(..), \
             restart(..), owned_by(..) and the rest",
        )
    } else {
        None
    }
}

/// A statement is made by its factory, never by its class: no `DDL` class
/// has a `#[new]`.
// [spec:pgorm:req:bindings.schema-api+2]
pub(crate) fn install(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyDataType>()?;
    module.add_class::<PyColumnDef>()?;
    module.add_class::<PyDDL>()?;
    module.add_class::<PyCreateTable>()?;
    module.add_class::<alter::PyPendingAlterTable>()?;
    module.add_class::<alter::PyAlterTable>()?;
    module.add_class::<PyCreateIndex>()?;
    module.add_class::<PyCreateType>()?;
    module.add_class::<custom_types::PyPendingAlterType>()?;
    module.add_class::<custom_types::PyAlterComposite>()?;
    module.add_class::<custom_types::PyRenameAttribute>()?;
    module.add_class::<domain::PyCreateDomain>()?;
    module.add_class::<domain::PyPendingAlterDomain>()?;
    module.add_class::<sequence::PyCreateSequence>()?;
    module.add_class::<sequence::PyPendingAlterSequence>()?;
    module.add_class::<sequence::PyAlterSequence>()?;
    module.add_class::<view::PyCreateView>()?;
    module.add_class::<view::PyCreateRecursiveView>()?;
    module.add_class::<view::PyCreateMaterializedView>()?;
    module.add_class::<view::PyCreateTableAs>()?;
    module.add_class::<PyEntitySchema>()?;
    module.add_class::<PyMigrationFile>()?;
    module.add_function(wrap_pyfunction!(migration_file::migration_file, module)?)?;
    module.add_function(wrap_pyfunction!(entity::schema_from_entity, module)?)?;
    module.add_function(wrap_pyfunction!(table::create_table, module)?)?;
    module.add_function(wrap_pyfunction!(index::create_index, module)?)?;
    module.add_function(wrap_pyfunction!(custom_types::create_type, module)?)?;
    module.add_function(wrap_pyfunction!(domain::create_domain, module)?)?;
    module.add_function(wrap_pyfunction!(sequence::create_sequence, module)?)?;
    module.add_function(wrap_pyfunction!(view::create_view, module)?)?;
    module.add_function(wrap_pyfunction!(view::create_recursive_view, module)?)?;
    module.add_function(wrap_pyfunction!(view::create_materialized_view, module)?)?;
    module.add_function(wrap_pyfunction!(view::create_table_as, module)?)?;
    module.add_function(wrap_pyfunction!(alter::alter_table, module)?)?;
    module.add_function(wrap_pyfunction!(table::drop_table, module)?)?;
    module.add_function(wrap_pyfunction!(table::rename_table, module)?)?;
    module.add_function(wrap_pyfunction!(table::rename_column, module)?)?;
    module.add_function(wrap_pyfunction!(table::rename_constraint, module)?)?;
    module.add_function(wrap_pyfunction!(table::truncate_table, module)?)?;
    module.add_function(wrap_pyfunction!(table::comment_on_table, module)?)?;
    module.add_function(wrap_pyfunction!(table::comment_on_column, module)?)?;
    module.add_function(wrap_pyfunction!(index::drop_index, module)?)?;
    module.add_function(wrap_pyfunction!(custom_types::alter_type, module)?)?;
    module.add_function(wrap_pyfunction!(custom_types::drop_type, module)?)?;
    module.add_function(wrap_pyfunction!(domain::alter_domain, module)?)?;
    module.add_function(wrap_pyfunction!(domain::drop_domain, module)?)?;
    module.add_function(wrap_pyfunction!(sequence::alter_sequence, module)?)?;
    module.add_function(wrap_pyfunction!(sequence::drop_sequence, module)?)?;
    module.add_function(wrap_pyfunction!(sequence::rename_sequence, module)?)?;
    module.add_function(wrap_pyfunction!(sequence::create_extension, module)?)?;
    module.add_function(wrap_pyfunction!(sequence::drop_extension, module)?)?;
    module.add_function(wrap_pyfunction!(namespace::create_schema, module)?)?;
    module.add_function(wrap_pyfunction!(namespace::drop_schema, module)?)?;
    module.add_function(wrap_pyfunction!(namespace::rename_schema, module)?)?;
    module.add_function(wrap_pyfunction!(view::drop_view, module)?)?;
    module.add_function(wrap_pyfunction!(view::refresh_materialized_view, module)?)?;
    module.add_function(wrap_pyfunction!(view::drop_materialized_view, module)?)?;
    Ok(())
}
