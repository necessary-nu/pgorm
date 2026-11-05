//! Sequences — created, altered over pgorm-query's typestate, dropped and
//! renamed — and extensions. A sequence's options are the vocabulary an
//! identity column's take, and a sequence is a relation, named as a table
//! is.

use super::{
    args::{self, integer, pick, relation, relations, sequence_options, text},
    drops::{drop_list, dropping},
    statement::{PyDDL, Statement, ddl_class},
};
use crate::errors::ConstructionError;
use pgorm::pgorm_query::{
    PendingSequenceAlter, Sequence, SequenceAlterStatement, SequenceCreateStatement, SequenceType,
    extension::Extension,
};
use pyo3::{prelude::*, types::PyDict};

enum Clauses {
    Create(SequenceCreateStatement),
    Pending(PendingSequenceAlter),
    Alter(SequenceAlterStatement),
}

/// Apply the clause `$method`, which a create and an alter share, to the
/// receiver: a pending alter becomes the statement it starts.
macro_rules! clause {
    ($slf:expr, $clauses:expr, $method:ident($($arg:expr),*)) => {{
        let py = $slf.py();
        Ok(match $clauses {
            Clauses::Create(mut create) => {
                create.$method($($arg),*);
                PyCreateSequence::object(py, create)?.into_any()
            }
            Clauses::Pending(pending) => {
                PyAlterSequence::object(py, pending.$method($($arg),*))?.into_any()
            }
            Clauses::Alter(mut alter) => {
                alter.$method($($arg),*);
                PyAlterSequence::object(py, alter)?.into_any()
            }
        })
    }};
}

const TYPES: &[(&str, SequenceType)] = &[
    ("smallint", SequenceType::SmallInteger),
    ("integer", SequenceType::Integer),
    ("bigint", SequenceType::BigInteger),
];

/// The clauses a sequence statement takes, creating the sequence or altering
/// it alike; `$receive` reads the receiver `$slf` as the statement it is.
macro_rules! sequence_clauses {
    ($class:ident, |$slf:ident| $receive:expr) => {
        #[pymethods]
        impl $class {
            /// The integer type the sequence counts in, one of the three
            /// PostgreSQL accepts.
            fn as_type<'py>(slf: &Bound<'py, Self>, kind: &str) -> PyResult<Bound<'py, PyAny>> {
                let kind = pick(kind, "a sequence's type", TYPES)?;
                let clauses = {
                    let $slf = slf;
                    $receive
                };
                clause!(slf, clauses, as_type(kind))
            }

            /// At least one option, merged into those set, a later one for a
            /// clause replacing the earlier.
            // [spec:pgorm:req:bindings.schema-sequences]
            #[pyo3(signature=(**options))]
            fn options<'py>(
                slf: &Bound<'py, Self>,
                options: Option<&Bound<'_, PyDict>>,
            ) -> PyResult<Bound<'py, PyAny>> {
                let Some(options) = sequence_options(options)? else {
                    return Err(ConstructionError::new_err(
                        "options(..) sets at least one option",
                    ));
                };
                let clauses = {
                    let $slf = slf;
                    $receive
                };
                clause!(slf, clauses, options(options))
            }

            /// `OWNED BY` a table's column, which takes the sequence with it
            /// when dropped, or with no table `OWNED BY NONE`, which releases
            /// it.
            // [spec:pgorm:req:bindings.schema-sequences]
            #[pyo3(signature=(table, column=None))]
            fn owned_by<'py>(
                slf: &Bound<'py, Self>,
                table: Option<&Bound<'_, PyAny>>,
                column: Option<&Bound<'_, PyAny>>,
            ) -> PyResult<Bound<'py, PyAny>> {
                let clauses = {
                    let $slf = slf;
                    $receive
                };
                match (table, column) {
                    (None, None) => clause!(slf, clauses, owned_by_none()),
                    (Some(table), Some(column)) => {
                        let (table, column) = (relation(table)?, args::name(column)?);
                        clause!(slf, clauses, owned_by(table, column))
                    }
                    _ => Err(ConstructionError::new_err(
                        "a sequence is owned by a table's column, both named, or by None",
                    )),
                }
            }
        }
    };
}

/// `RESTART`, at `value` when one is given: an alter's alone, so the
/// receiver `$slf`, which `$receive` reads, is an alter, pending or not.
macro_rules! restart {
    ($class:ident, |$slf:ident| $receive:expr) => {
        #[pymethods]
        impl $class {
            #[pyo3(signature=(value=None))]
            fn restart<'py>(
                slf: &Bound<'py, Self>,
                value: Option<&Bound<'_, PyAny>>,
            ) -> PyResult<Bound<'py, PyAlterSequence>> {
                let value = value
                    .map(|value| integer(value, "a restart value"))
                    .transpose()?;
                let clauses = {
                    let $slf = slf;
                    $receive
                };
                let statement = match (clauses, value) {
                    (Clauses::Pending(pending), Some(value)) => pending.restart_with(value),
                    (Clauses::Pending(pending), None) => pending.restart(),
                    (Clauses::Alter(mut alter), Some(value)) => {
                        alter.restart_with(value);
                        alter
                    }
                    (Clauses::Alter(mut alter), None) => {
                        alter.restart();
                        alter
                    }
                    (Clauses::Create(_), _) => {
                        return Err(ConstructionError::new_err(
                            "RESTART belongs to an ALTER SEQUENCE",
                        ));
                    }
                };
                PyAlterSequence::object(slf.py(), statement)
            }
        }
    };
}

/// `CREATE SEQUENCE`; every method gives a new statement.
#[derive(Debug)]
#[pyclass(name = "CreateSequence", module = "pgorm.schema", frozen, extends = PyDDL)]
pub struct PyCreateSequence;

ddl_class!(PyCreateSequence, CreateSequence, SequenceCreateStatement);

// [spec:pgorm:req:bindings.schema-sequences]
#[pyfunction]
pub(super) fn create_sequence<'py>(
    py: Python<'py>,
    name: &Bound<'_, PyAny>,
) -> PyResult<Bound<'py, PyCreateSequence>> {
    PyCreateSequence::object(py, Sequence::create(relation(name)?))
}

#[pymethods]
impl PyCreateSequence {
    fn if_not_exists<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        let mut statement = Self::statement(slf)?;
        statement.if_not_exists();
        Self::object(slf.py(), statement)
    }
}

sequence_clauses!(PyCreateSequence, |slf| Clauses::Create(
    PyCreateSequence::statement(slf)?
));

/// An `ALTER SEQUENCE` before its first clause. PostgreSQL parses none
/// without one, so it has no `inspect()`, and a terminal refuses it.
#[derive(Debug)]
#[pyclass(name = "PendingAlterSequence", module = "pgorm.schema", frozen)]
pub struct PyPendingAlterSequence {
    pending: PendingSequenceAlter,
}

#[pymethods]
impl PyPendingAlterSequence {
    // [spec:pgorm:req:python.truth-testing+4]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "an ALTER SEQUENCE with no clause cannot be tested as a Python boolean",
        ))
    }
}

sequence_clauses!(PyPendingAlterSequence, |slf| Clauses::Pending(
    slf.get().pending.clone()
));
restart!(PyPendingAlterSequence, |slf| Clauses::Pending(
    slf.get().pending.clone()
));

/// An `ALTER SEQUENCE` with at least one clause; every method gives a new
/// one.
#[derive(Debug)]
#[pyclass(name = "AlterSequence", module = "pgorm.schema", frozen, extends = PyDDL)]
pub struct PyAlterSequence;

ddl_class!(PyAlterSequence, AlterSequence, SequenceAlterStatement);

#[pymethods]
impl PyAlterSequence {
    fn if_exists<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        let mut statement = Self::statement(slf)?;
        statement.if_exists();
        Self::object(slf.py(), statement)
    }
}

sequence_clauses!(PyAlterSequence, |slf| Clauses::Alter(
    PyAlterSequence::statement(slf)?
));
restart!(PyAlterSequence, |slf| Clauses::Alter(
    PyAlterSequence::statement(slf)?
));

/// `alter_sequence(name)`: the sequence, with no clause yet.
// [spec:pgorm:req:bindings.schema-sequences]
#[pyfunction]
pub(super) fn alter_sequence(name: &Bound<'_, PyAny>) -> PyResult<PyPendingAlterSequence> {
    Ok(PyPendingAlterSequence {
        pending: Sequence::alter(relation(name)?),
    })
}

/// `DROP SEQUENCE` over one sequence or a non-empty list of them.
#[pyfunction]
#[pyo3(signature=(names, *, if_exists=false, behavior=None))]
pub(super) fn drop_sequence(
    names: &Bound<'_, PyAny>,
    if_exists: bool,
    behavior: Option<&str>,
) -> PyResult<PyDDL> {
    let drop = drop_list(relations(names)?, if_exists, behavior)?;
    Ok(PyDDL::new(Statement::DropSequence(drop)))
}

#[pyfunction]
pub(super) fn rename_sequence(
    name: &Bound<'_, PyAny>,
    new_name: &Bound<'_, PyAny>,
) -> PyResult<PyDDL> {
    let renamed = Sequence::rename(relation(name)?, args::name(new_name)?);
    Ok(PyDDL::new(Statement::RenameSequence(renamed)))
}

/// `CREATE EXTENSION`: the name and schema quoted, the version a literal.
// [spec:pgorm:req:bindings.schema-sequences]
#[pyfunction]
#[pyo3(signature=(name, *, if_not_exists=false, schema=None, version=None, cascade=false))]
pub(super) fn create_extension(
    name: &Bound<'_, PyAny>,
    if_not_exists: bool,
    schema: Option<&Bound<'_, PyAny>>,
    version: Option<&Bound<'_, PyAny>>,
    cascade: bool,
) -> PyResult<PyDDL> {
    let mut create = Extension::create(args::name(name)?);
    if if_not_exists {
        create.if_not_exists();
    }
    if let Some(schema) = schema {
        create.schema(args::name(schema)?);
    }
    if let Some(version) = version {
        create.version(text(version, "an extension's version")?);
    }
    if cascade {
        create.cascade();
    }
    Ok(PyDDL::new(Statement::CreateExtension(create)))
}

#[pyfunction]
#[pyo3(signature=(name, *, if_exists=false, behavior=None))]
pub(super) fn drop_extension(
    name: &Bound<'_, PyAny>,
    if_exists: bool,
    behavior: Option<&str>,
) -> PyResult<PyDDL> {
    let drop = dropping(Extension::drop(args::name(name)?), if_exists, behavior)?;
    Ok(PyDDL::new(Statement::DropExtension(drop)))
}
