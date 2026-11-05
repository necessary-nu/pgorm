//! `ALTER TABLE` over pgorm-query's typestate: `alter_table(table)` names the
//! table and nothing more, as `PendingTableAlter` does, and its first action
//! gives the statement, which takes more. Each action is the pgorm-query
//! method of the same name, on the pending alter or the statement alike, so
//! the two classes take the one set of actions written below.

use super::{
    args::{self, behavior, pick, relation},
    column::{self, PyColumnDef},
    keys::{self, ForeignKeyOptions},
    statement::{AlteredTable, PyDDL, ddl_class},
};
use crate::{errors::ConstructionError, expressions};
use pgorm::pgorm_query::{
    ColumnDef, ColumnSpec, ConstraintChange, ConstraintDrop, DropBehavior, NotNullConstraint,
    PendingTableAlter, Primary, Table, TableAlterStatement, TableName, Unique,
};
use pyo3::prelude::*;

enum Alter {
    Pending(PendingTableAlter),
    Statement(TableAlterStatement),
}

/// Apply the pgorm-query action `$method` to an alter, pending or not, giving
/// the `AlterTable` it makes.
macro_rules! act {
    ($slf:expr, $method:ident($($arg:expr),*)) => {{
        let (table, alter) = Self::alter($slf)?;
        let statement = match alter {
            Alter::Pending(pending) => pending.$method($($arg),*),
            Alter::Statement(mut statement) => {
                statement.$method($($arg),*);
                statement
            }
        };
        PyAlterTable::object($slf.py(), AlteredTable { table, statement })
    }};
}

/// An `ALTER TABLE` before its first action. PostgreSQL parses none without
/// one, so it has no `inspect()`, and a terminal refuses it.
#[derive(Debug)]
#[pyclass(name = "PendingAlterTable", module = "pgorm.schema", frozen)]
pub struct PyPendingAlterTable {
    table: TableName,
    pending: PendingTableAlter,
}

impl PyPendingAlterTable {
    fn alter(slf: &Bound<'_, Self>) -> PyResult<(TableName, Alter)> {
        let this = slf.get();
        Ok((this.table.clone(), Alter::Pending(this.pending.clone())))
    }
}

#[pymethods]
impl PyPendingAlterTable {
    // [spec:pgorm:req:python.truth-testing+4]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "an ALTER TABLE with no action cannot be tested as a Python boolean",
        ))
    }
}

/// An `ALTER TABLE` with at least one action, applied in order; every method
/// gives a new one.
#[derive(Debug)]
#[pyclass(name = "AlterTable", module = "pgorm.schema", frozen, extends = PyDDL)]
pub struct PyAlterTable;

ddl_class!(PyAlterTable, AlterTable, AlteredTable);

impl PyAlterTable {
    fn alter(slf: &Bound<'_, Self>) -> PyResult<(TableName, Alter)> {
        let altered = Self::statement(slf)?;
        Ok((altered.table, Alter::Statement(altered.statement)))
    }
}

/// `alter_table(table)`: the table, with no action yet.
// [spec:pgorm:req:bindings.schema-alter]
#[pyfunction]
pub(super) fn alter_table(table: &Bound<'_, PyAny>) -> PyResult<PyPendingAlterTable> {
    let table = relation(table)?;
    Ok(PyPendingAlterTable {
        pending: Table::alter(table.clone()),
        table,
    })
}

/// The column `modify_column` changes. pgorm-query writes each aspect it
/// carries as an action of its own; one it would leave out — a generated
/// expression, the serial family, a collation with no type to retype — is
/// refused rather than dropped, and so is a column that changes nothing.
// [spec:pgorm:req:bindings.schema-alter]
fn modified(column: &PyColumnDef) -> PyResult<ColumnDef> {
    let column = &column.inner;
    for spec in column.get_column_spec() {
        match spec {
            ColumnSpec::Generated { .. } => {
                return Err(ConstructionError::new_err(
                    "a modified column cannot change its generated expression: use \
                     set_expression(..) or drop_expression(..)",
                ));
            }
            ColumnSpec::AutoIncrement => {
                return Err(ConstructionError::new_err(
                    "a modified column cannot take the serial family: use identity(..)",
                ));
            }
            _ => {}
        }
    }
    if column.get_collation().is_some() && column.get_column_type().is_none() {
        return Err(ConstructionError::new_err(
            "a modified column changes its collation only with its type, which it does not give",
        ));
    }
    if column.get_column_type().is_none() && column.get_column_spec().is_empty() {
        return Err(ConstructionError::new_err(
            "a modified column changes something: its type, nullability, default, a CHECK or an \
             identity",
        ));
    }
    Ok(column.clone())
}

fn constraint_drop(
    name: &Bound<'_, PyAny>,
    if_exists: bool,
    behavior: Option<&str>,
) -> PyResult<ConstraintDrop> {
    let mut drop = ConstraintDrop::new(args::name(name)?);
    if if_exists {
        drop = drop.if_exists();
    }
    Ok(match self::behavior(behavior)? {
        Some(DropBehavior::Cascade) => drop.cascade(),
        Some(DropBehavior::Restrict) => drop.restrict(),
        None => drop,
    })
}

fn not_null(
    column: &Bound<'_, PyAny>,
    name: Option<&Bound<'_, PyAny>>,
    no_inherit: bool,
    not_valid: bool,
) -> PyResult<NotNullConstraint> {
    let mut constraint = NotNullConstraint::new(args::name(column)?);
    if let Some(name) = name {
        constraint = constraint.name(args::name(name)?);
    }
    if no_inherit {
        constraint = constraint.no_inherit();
    }
    if not_valid {
        constraint = constraint.not_valid();
    }
    Ok(constraint)
}

const CHANGES: &[(&str, ConstraintChange)] = &[
    ("inherit", ConstraintChange::Inherit),
    ("no_inherit", ConstraintChange::NoInherit),
    ("enforced", ConstraintChange::Enforced),
    ("not_enforced", ConstraintChange::NotEnforced),
];

/// The actions an `ALTER TABLE` takes, before its first and after it alike,
/// each giving an `AlterTable`.
macro_rules! actions {
    ($class:ident) => {
        #[pymethods]
        impl $class {
            #[pyo3(signature=(column, *, if_not_exists=false))]
            fn add_column<'py>(
                slf: &Bound<'py, Self>,
                column: &PyColumnDef,
                if_not_exists: bool,
            ) -> PyResult<Bound<'py, PyAlterTable>> {
                let column = column.typed("an added column")?;
                if if_not_exists {
                    act!(slf, add_column_if_not_exists(column))
                } else {
                    act!(slf, add_column(column))
                }
            }

            /// The column's type, nullability, default, `CHECK` or identity
            /// changed, each as pgorm-query writes it.
            // [spec:pgorm:req:bindings.schema-alter]
            fn modify_column<'py>(
                slf: &Bound<'py, Self>,
                column: &PyColumnDef,
            ) -> PyResult<Bound<'py, PyAlterTable>> {
                let column = modified(column)?;
                act!(slf, modify_column(column))
            }

            fn drop_column<'py>(
                slf: &Bound<'py, Self>,
                name: &Bound<'_, PyAny>,
            ) -> PyResult<Bound<'py, PyAlterTable>> {
                let name = args::name(name)?;
                act!(slf, drop_column(name))
            }

            #[pyo3(signature=(columns, *, name=None, include=None, deferrability=None, without_overlaps=None))]
            fn add_primary_key<'py>(
                slf: &Bound<'py, Self>,
                columns: &Bound<'_, PyAny>,
                name: Option<&Bound<'_, PyAny>>,
                include: Option<&Bound<'_, PyAny>>,
                deferrability: Option<&str>,
                without_overlaps: Option<&Bound<'_, PyAny>>,
            ) -> PyResult<Bound<'py, PyAlterTable>> {
                let key =
                    keys::key::<Primary>(columns, name, include, deferrability, without_overlaps)?;
                act!(slf, add_primary_key(key))
            }

            #[pyo3(signature=(columns, *, name=None, include=None, deferrability=None, without_overlaps=None, nulls_not_distinct=false))]
            #[allow(clippy::too_many_arguments)]
            fn add_unique<'py>(
                slf: &Bound<'py, Self>,
                columns: &Bound<'_, PyAny>,
                name: Option<&Bound<'_, PyAny>>,
                include: Option<&Bound<'_, PyAny>>,
                deferrability: Option<&str>,
                without_overlaps: Option<&Bound<'_, PyAny>>,
                nulls_not_distinct: bool,
            ) -> PyResult<Bound<'py, PyAlterTable>> {
                let key =
                    keys::key::<Unique>(columns, name, include, deferrability, without_overlaps)?;
                let key = keys::unique(key, nulls_not_distinct);
                act!(slf, add_unique(key))
            }

            /// A foreign key, `NOT VALID` when asked, which leaves the rows
            /// already there unchecked.
            // [spec:pgorm:req:bindings.schema-alter]
            #[pyo3(signature=(columns, references, ref_columns, *, name=None, on_delete=None, on_update=None, deferrability=None, enforcement=None, period=None, not_valid=false))]
            #[allow(clippy::too_many_arguments)]
            fn add_foreign_key<'py>(
                slf: &Bound<'py, Self>,
                columns: &Bound<'_, PyAny>,
                references: &Bound<'_, PyAny>,
                ref_columns: &Bound<'_, PyAny>,
                name: Option<&Bound<'_, PyAny>>,
                on_delete: Option<&str>,
                on_update: Option<&str>,
                deferrability: Option<&str>,
                enforcement: Option<&str>,
                period: Option<&Bound<'_, PyAny>>,
                not_valid: bool,
            ) -> PyResult<Bound<'py, PyAlterTable>> {
                let (table, _) = Self::alter(slf)?;
                let options = ForeignKeyOptions {
                    name,
                    on_delete,
                    on_update,
                    deferrability,
                    enforcement,
                    period,
                };
                let key = keys::foreign_key(table, columns, references, ref_columns, options)?;
                let key = key.get_foreign_key().clone();
                if not_valid {
                    act!(slf, add_foreign_key(key.not_valid()))
                } else {
                    act!(slf, add_foreign_key(key))
                }
            }

            #[pyo3(signature=(condition, *, name=None, enforcement=None, no_inherit=false, not_valid=false))]
            fn add_check<'py>(
                slf: &Bound<'py, Self>,
                condition: &Bound<'_, PyAny>,
                name: Option<&Bound<'_, PyAny>>,
                enforcement: Option<&str>,
                no_inherit: bool,
                not_valid: bool,
            ) -> PyResult<Bound<'py, PyAlterTable>> {
                let check = column::check(condition, name, enforcement, no_inherit)?;
                if not_valid {
                    act!(slf, add_check(check.not_valid()))
                } else {
                    act!(slf, add_check(check))
                }
            }

            /// PostgreSQL 18's table-level `NOT NULL`, the one spelling with a
            /// place for `NOT VALID`.
            // [spec:pgorm:req:bindings.schema-alter]
            #[pyo3(signature=(column, *, name=None, no_inherit=false, not_valid=false))]
            fn add_not_null<'py>(
                slf: &Bound<'py, Self>,
                column: &Bound<'_, PyAny>,
                name: Option<&Bound<'_, PyAny>>,
                no_inherit: bool,
                not_valid: bool,
            ) -> PyResult<Bound<'py, PyAlterTable>> {
                let constraint = not_null(column, name, no_inherit, not_valid)?;
                act!(slf, add_not_null(constraint))
            }

            /// A constraint of any kind, by name: which kind it is, is the
            /// server's knowledge.
            #[pyo3(signature=(name, *, if_exists=false, behavior=None))]
            fn drop_constraint<'py>(
                slf: &Bound<'py, Self>,
                name: &Bound<'_, PyAny>,
                if_exists: bool,
                behavior: Option<&str>,
            ) -> PyResult<Bound<'py, PyAlterTable>> {
                let drop = constraint_drop(name, if_exists, behavior)?;
                act!(slf, drop_constraint(drop))
            }

            fn validate_constraint<'py>(
                slf: &Bound<'py, Self>,
                name: &Bound<'_, PyAny>,
            ) -> PyResult<Bound<'py, PyAlterTable>> {
                let name = args::name(name)?;
                act!(slf, validate_constraint(name))
            }

            /// Whether a `NOT NULL` passes to inheriting tables, or whether a
            /// foreign key is enforced.
            // [spec:pgorm:req:bindings.schema-alter]
            fn alter_constraint<'py>(
                slf: &Bound<'py, Self>,
                name: &Bound<'_, PyAny>,
                change: &str,
            ) -> PyResult<Bound<'py, PyAlterTable>> {
                let name = args::name(name)?;
                let change = pick(change, "a constraint's change", CHANGES)?;
                act!(slf, alter_constraint(name, change))
            }

            /// A generated column's new expression, which the rows already
            /// written take.
            fn set_expression<'py>(
                slf: &Bound<'py, Self>,
                column: &Bound<'_, PyAny>,
                expression: &Bound<'_, PyAny>,
            ) -> PyResult<Bound<'py, PyAlterTable>> {
                let column = args::name(column)?;
                let expression = expressions::require_expr(expression)?.inner;
                act!(slf, set_expression(column, expression))
            }

            /// A stored generated column made plain, keeping its values.
            #[pyo3(signature=(column, *, if_exists=false))]
            fn drop_expression<'py>(
                slf: &Bound<'py, Self>,
                column: &Bound<'_, PyAny>,
                if_exists: bool,
            ) -> PyResult<Bound<'py, PyAlterTable>> {
                let column = args::name(column)?;
                if if_exists {
                    act!(slf, drop_expression_if_exists(column))
                } else {
                    act!(slf, drop_expression(column))
                }
            }
        }
    };
}

actions!(PyPendingAlterTable);
actions!(PyAlterTable);
