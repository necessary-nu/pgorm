//! `CREATE TYPE` — a shell type, an enumeration, a composite or a range —
//! `ALTER TYPE` over pgorm-query's typestate, and `DROP TYPE`. What a type
//! is, is one slot: choosing a kind replaces the one before, as pgorm-query's
//! builder does.

use super::{
    args::{self, behavior, collation, label, type_ref, type_refs},
    drops::drop_list,
    statement::{PyDDL, Statement, ddl_class},
    types::PyDataType,
};
use crate::errors::ConstructionError;
use pgorm::pgorm_query::{
    DropBehavior,
    extension::{
        AttributeRenameStatement, CompositeAlterStatement, PendingTypeAlter, RangeDefinition, Type,
        TypeCreateStatement,
    },
};
use pyo3::{exceptions::PyTypeError, prelude::*};

/// `CREATE TYPE`: a shell type until a kind is chosen; every method gives a
/// new statement.
#[derive(Debug)]
#[pyclass(name = "CreateType", module = "pgorm.schema", frozen, extends = PyDDL)]
pub struct PyCreateType;

ddl_class!(PyCreateType, CreateType, TypeCreateStatement, with);

/// `CREATE TYPE` naming its type, a shell type until a kind is chosen.
// [spec:pgorm:req:bindings.schema-types]
#[pyfunction]
pub(super) fn create_type<'py>(
    py: Python<'py>,
    name: &Bound<'_, PyAny>,
) -> PyResult<Bound<'py, PyCreateType>> {
    PyCreateType::object(py, Type::create(type_ref(name)?))
}

#[pymethods]
impl PyCreateType {
    /// An enumeration, with no labels until `values` appends them.
    fn as_enum<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        let mut statement = Self::statement(slf)?;
        statement.as_enum();
        Self::object(slf.py(), statement)
    }

    /// Labels appended to an enumeration, which the type becomes: data,
    /// written as literals.
    // [spec:pgorm:req:bindings.schema-types]
    fn values<'py>(
        slf: &Bound<'py, Self>,
        labels: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        let labels = labels
            .extract::<Vec<Bound<'_, PyAny>>>()
            .map_err(|_| PyTypeError::new_err("an enumeration's labels are a list of str"))?
            .iter()
            .map(label)
            .collect::<PyResult<Vec<_>>>()?;
        Self::with(slf, |statement| {
            statement.values(labels);
        })
    }

    /// A composite, with no attributes until `attribute` appends them.
    fn as_composite<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        let mut statement = Self::statement(slf)?;
        statement.as_composite();
        Self::object(slf.py(), statement)
    }

    /// An attribute appended to a composite, which the type becomes.
    // [spec:pgorm:req:bindings.schema-types]
    #[pyo3(signature=(name, kind, *, collation=None))]
    fn attribute<'py>(
        slf: &Bound<'py, Self>,
        name: &Bound<'_, PyAny>,
        kind: &Bound<'_, PyAny>,
        collation: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, Self>> {
        let name = args::name(name)?;
        let kind = PyDataType::coerce(kind)?.inner;
        let collation = self::collation(collation)?;
        Self::with(slf, |statement| {
            match collation {
                Some(collation) => statement.attribute_collated(name, kind, collation),
                None => statement.attribute(name, kind),
            };
        })
    }

    /// A range over `subtype`, the one option PostgreSQL requires, with the
    /// operator class, collation, difference function and multirange name
    /// given.
    // [spec:pgorm:req:bindings.schema-types]
    #[pyo3(signature=(subtype, *, collation=None, subtype_opclass=None, subtype_diff=None, multirange_type_name=None))]
    fn as_range<'py>(
        slf: &Bound<'py, Self>,
        subtype: &Bound<'_, PyAny>,
        collation: Option<&Bound<'_, PyAny>>,
        subtype_opclass: Option<&Bound<'_, PyAny>>,
        subtype_diff: Option<&Bound<'_, PyAny>>,
        multirange_type_name: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, Self>> {
        let mut range = RangeDefinition::new(PyDataType::coerce(subtype)?.inner);
        if let Some(opclass) = subtype_opclass {
            range = range.subtype_opclass(args::name(opclass)?);
        }
        if let Some(collation) = self::collation(collation)? {
            range = range.collation(collation);
        }
        if let Some(function) = subtype_diff {
            range = range.subtype_diff(args::name(function)?);
        }
        if let Some(name) = multirange_type_name {
            range = range.multirange_type_name(type_ref(name)?);
        }
        Self::with(slf, |statement| {
            statement.as_range(range);
        })
    }
}

enum Composite {
    Pending(PendingTypeAlter),
    Statement(CompositeAlterStatement),
}

/// Apply the pgorm-query change `$method` to a composite's alteration, which
/// on a pending alter consumes it and on a statement appends.
macro_rules! change {
    ($slf:expr, $composite:expr, $method:ident($($arg:expr),*)) => {{
        let statement = match $composite {
            Composite::Pending(pending) => pending.$method($($arg),*),
            Composite::Statement(statement) => statement.$method($($arg),*),
        };
        PyAlterComposite::object($slf.py(), statement)
    }};
}

/// The attribute changes a composite's `ALTER TYPE` takes, before its first
/// and after it alike, each giving an `AlterComposite`; `$receive` reads the
/// receiver `$slf` as the alteration it is.
macro_rules! attribute_changes {
    ($class:ident, |$slf:ident| $receive:expr) => {
        #[pymethods]
        impl $class {
            // [spec:pgorm:req:bindings.schema-types]
            #[pyo3(signature=(name, kind, *, collation=None))]
            fn add_attribute<'py>(
                slf: &Bound<'py, Self>,
                name: &Bound<'_, PyAny>,
                kind: &Bound<'_, PyAny>,
                collation: Option<&Bound<'_, PyAny>>,
            ) -> PyResult<Bound<'py, PyAlterComposite>> {
                let name = args::name(name)?;
                let kind = PyDataType::coerce(kind)?.inner;
                let composite = {
                    let $slf = slf;
                    $receive
                };
                match self::collation(collation)? {
                    Some(collation) => {
                        change!(
                            slf,
                            composite,
                            add_attribute_collated(name, kind, collation)
                        )
                    }
                    None => change!(slf, composite, add_attribute(name, kind)),
                }
            }

            #[pyo3(signature=(name, *, if_exists=false))]
            fn drop_attribute<'py>(
                slf: &Bound<'py, Self>,
                name: &Bound<'_, PyAny>,
                if_exists: bool,
            ) -> PyResult<Bound<'py, PyAlterComposite>> {
                let name = args::name(name)?;
                let composite = {
                    let $slf = slf;
                    $receive
                };
                if if_exists {
                    change!(slf, composite, drop_attribute_if_exists(name))
                } else {
                    change!(slf, composite, drop_attribute(name))
                }
            }

            #[pyo3(signature=(name, kind, *, collation=None))]
            fn alter_attribute<'py>(
                slf: &Bound<'py, Self>,
                name: &Bound<'_, PyAny>,
                kind: &Bound<'_, PyAny>,
                collation: Option<&Bound<'_, PyAny>>,
            ) -> PyResult<Bound<'py, PyAlterComposite>> {
                let name = args::name(name)?;
                let kind = PyDataType::coerce(kind)?.inner;
                let composite = {
                    let $slf = slf;
                    $receive
                };
                match self::collation(collation)? {
                    Some(collation) => {
                        change!(
                            slf,
                            composite,
                            alter_attribute_collated(name, kind, collation)
                        )
                    }
                    None => change!(slf, composite, alter_attribute(name, kind)),
                }
            }
        }
    };
}

/// An `ALTER TYPE` before its change. PostgreSQL parses none without one, so
/// it has no `inspect()`, and a terminal refuses it.
#[derive(Debug)]
#[pyclass(name = "PendingAlterType", module = "pgorm.schema", frozen)]
pub struct PyPendingAlterType {
    pending: PendingTypeAlter,
}

#[pymethods]
impl PyPendingAlterType {
    /// A label added to an enumeration, at its end or beside one, never both.
    // [spec:pgorm:req:bindings.schema-types]
    #[pyo3(signature=(label, *, before=None, after=None))]
    fn add_value(
        &self,
        label: &Bound<'_, PyAny>,
        before: Option<&Bound<'_, PyAny>>,
        after: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyDDL> {
        let statement = self.pending.clone().add_value(args::label(label)?);
        let statement = match (before, after) {
            (Some(_), Some(_)) => {
                return Err(ConstructionError::new_err(
                    "a label is added before one label or after one, not both",
                ));
            }
            (Some(before), None) => statement.before(args::label(before)?),
            (None, Some(after)) => statement.after(args::label(after)?),
            (None, None) => statement,
        };
        Ok(PyDDL::new(Statement::AlterType(statement)))
    }

    /// The type renamed; the new name is bare, the type staying in its
    /// schema.
    fn rename_to(&self, name: &Bound<'_, PyAny>) -> PyResult<PyDDL> {
        let statement = self.pending.clone().rename_to(args::name(name)?);
        Ok(PyDDL::new(Statement::AlterType(statement)))
    }

    fn rename_value(
        &self,
        label: &Bound<'_, PyAny>,
        new_label: &Bound<'_, PyAny>,
    ) -> PyResult<PyDDL> {
        let (from, to) = (args::label(label)?, args::label(new_label)?);
        let statement = self.pending.clone().rename_value(from, to);
        Ok(PyDDL::new(Statement::AlterType(statement)))
    }

    /// An attribute renamed: PostgreSQL takes `RENAME` as an `ALTER TYPE`'s
    /// sole action, so it is a statement of its own.
    // [spec:pgorm:req:bindings.schema-types]
    fn rename_attribute<'py>(
        slf: &Bound<'py, Self>,
        name: &Bound<'_, PyAny>,
        new_name: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyRenameAttribute>> {
        let (from, to) = (args::name(name)?, args::name(new_name)?);
        let statement = slf.get().pending.clone().rename_attribute(from, to);
        PyRenameAttribute::object(slf.py(), statement)
    }

    // [spec:pgorm:req:python.truth-testing+4]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "an ALTER TYPE with no change cannot be tested as a Python boolean",
        ))
    }
}

attribute_changes!(PyPendingAlterType, |slf| Composite::Pending(
    slf.get().pending.clone()
));

/// A composite's attribute changes, applied in order; every method gives a
/// new statement.
#[derive(Debug)]
#[pyclass(name = "AlterComposite", module = "pgorm.schema", frozen, extends = PyDDL)]
pub struct PyAlterComposite;

ddl_class!(PyAlterComposite, AlterComposite, CompositeAlterStatement);

#[pymethods]
impl PyAlterComposite {
    /// `CASCADE`, which carries the changes into typed tables, or
    /// `RESTRICT`; the last call wins.
    // [spec:pgorm:req:bindings.schema-types]
    fn behavior<'py>(slf: &Bound<'py, Self>, behavior: &str) -> PyResult<Bound<'py, Self>> {
        let statement = Self::statement(slf)?;
        let statement = match self::behavior(Some(behavior))? {
            Some(DropBehavior::Cascade) => statement.cascade(),
            _ => statement.restrict(),
        };
        Self::object(slf.py(), statement)
    }
}

attribute_changes!(PyAlterComposite, |slf| Composite::Statement(
    PyAlterComposite::statement(slf)?
));

/// A composite's attribute renamed.
#[derive(Debug)]
#[pyclass(name = "RenameAttribute", module = "pgorm.schema", frozen, extends = PyDDL)]
pub struct PyRenameAttribute;

ddl_class!(PyRenameAttribute, RenameAttribute, AttributeRenameStatement);

#[pymethods]
impl PyRenameAttribute {
    fn behavior<'py>(slf: &Bound<'py, Self>, behavior: &str) -> PyResult<Bound<'py, Self>> {
        let statement = Self::statement(slf)?;
        let statement = match self::behavior(Some(behavior))? {
            Some(DropBehavior::Cascade) => statement.cascade(),
            _ => statement.restrict(),
        };
        Self::object(slf.py(), statement)
    }
}

/// `alter_type(name)`: the type, with no change yet.
// [spec:pgorm:req:bindings.schema-types]
#[pyfunction]
pub(super) fn alter_type(name: &Bound<'_, PyAny>) -> PyResult<PyPendingAlterType> {
    Ok(PyPendingAlterType {
        pending: Type::alter(type_ref(name)?),
    })
}

/// `DROP TYPE` over one type or a non-empty list of them.
// [spec:pgorm:req:bindings.schema-types]
#[pyfunction]
#[pyo3(signature=(names, *, if_exists=false, behavior=None))]
pub(super) fn drop_type(
    names: &Bound<'_, PyAny>,
    if_exists: bool,
    behavior: Option<&str>,
) -> PyResult<PyDDL> {
    let drop = drop_list(type_refs(names)?, if_exists, behavior)?;
    Ok(PyDDL::new(Statement::DropType(drop)))
}
