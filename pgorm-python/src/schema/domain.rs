//! `CREATE DOMAIN`, `ALTER DOMAIN` over pgorm-query's typestate, and `DROP
//! DOMAIN`. A domain is a type, so it is named as one is: one identifier, or
//! a schema-qualified `TypeName`.

use super::{
    args::{self, type_ref, type_refs},
    drops::drop_list,
    statement::{PyDDL, Statement, ddl_class},
    types::PyDataType,
};
use crate::{errors::ConstructionError, expressions, identifiers::collation};
use pgorm::pgorm_query::{
    ConstraintDrop, Domain, DomainAlterStatement, DomainCheck, DomainCreateStatement, DropBehavior,
    PendingDomainAlter,
};
use pyo3::prelude::*;

fn check(condition: &Bound<'_, PyAny>, name: Option<&Bound<'_, PyAny>>) -> PyResult<DomainCheck> {
    let check = DomainCheck::new(expressions::require_expr(condition)?.inner);
    Ok(match name {
        Some(name) => check.name(args::name(name)?),
        None => check,
    })
}

/// `CREATE DOMAIN`; every method gives a new statement, and the clauses
/// render in one order whatever order they are given in.
#[derive(Debug)]
#[pyclass(name = "CreateDomain", module = "pgorm.schema", frozen, extends = PyDDL)]
pub struct PyCreateDomain;

ddl_class!(PyCreateDomain, CreateDomain, DomainCreateStatement, with);

/// A domain over `base`: any column type, another domain's `TypeName`
/// included.
// [spec:pgorm:req:bindings.schema-domains+2]
#[pyfunction]
pub(super) fn create_domain<'py>(
    py: Python<'py>,
    name: &Bound<'_, PyAny>,
    base: &Bound<'_, PyAny>,
) -> PyResult<Bound<'py, PyCreateDomain>> {
    let created = Domain::create(type_ref(name)?, PyDataType::coerce(base)?.inner);
    PyCreateDomain::object(py, created)
}

#[pymethods]
impl PyCreateDomain {
    // [spec:pgorm:req:bindings.schema-domains+2]
    #[pyo3(signature=(name, *, schema=None))]
    fn collate<'py>(
        slf: &Bound<'py, Self>,
        name: &Bound<'_, PyAny>,
        schema: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, Self>> {
        let named = collation(name, schema)?;
        Self::with(slf, |statement| {
            statement.collate(named);
        })
    }

    /// `DEFAULT`, one slot: a later call replaces an earlier one.
    fn default<'py>(
        slf: &Bound<'py, Self>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        let value = expressions::coerce(value)?.inner;
        Self::with(slf, |statement| {
            statement.default(value);
        })
    }

    /// `NOT NULL`, sharing one slot with `null()`, the later call winning, as
    /// PostgreSQL refuses the two together.
    fn not_null<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        let mut statement = Self::statement(slf)?;
        statement.not_null();
        Self::object(slf.py(), statement)
    }

    fn null<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        let mut statement = Self::statement(slf)?;
        statement.null();
        Self::object(slf.py(), statement)
    }

    /// A check appended, which takes no `not_valid`: PostgreSQL has no `NOT
    /// VALID` at `CREATE DOMAIN`.
    // [spec:pgorm:req:bindings.schema-domains+2]
    #[pyo3(signature=(condition, *, name=None))]
    fn check<'py>(
        slf: &Bound<'py, Self>,
        condition: &Bound<'_, PyAny>,
        name: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, Self>> {
        let check = check(condition, name)?;
        Self::with(slf, |statement| {
            statement.check(check);
        })
    }
}

/// An `ALTER DOMAIN` before its action. PostgreSQL parses none without one,
/// nor one with two, so it has no `inspect()`, a terminal refuses it, and
/// each action gives the whole statement. `ALTER DOMAIN` has no `IF EXISTS`.
#[derive(Debug)]
#[pyclass(name = "PendingAlterDomain", module = "pgorm.schema", frozen)]
pub struct PyPendingAlterDomain {
    pending: PendingDomainAlter,
}

impl PyPendingAlterDomain {
    fn altered(&self, action: impl FnOnce(PendingDomainAlter) -> DomainAlterStatement) -> PyDDL {
        PyDDL::new(Statement::AlterDomain(action(self.pending.clone())))
    }
}

#[pymethods]
impl PyPendingAlterDomain {
    // [spec:pgorm:req:bindings.schema-domains+2]
    fn set_default(&self, value: &Bound<'_, PyAny>) -> PyResult<PyDDL> {
        let value = expressions::coerce(value)?.inner;
        Ok(self.altered(|pending| pending.set_default(value)))
    }

    fn drop_default(&self) -> PyDDL {
        self.altered(PendingDomainAlter::drop_default)
    }

    fn set_not_null(&self) -> PyDDL {
        self.altered(PendingDomainAlter::set_not_null)
    }

    fn drop_not_null(&self) -> PyDDL {
        self.altered(PendingDomainAlter::drop_not_null)
    }

    /// A `CHECK` added: checked against the values already held, or,
    /// `not_valid`, left unchecked until `validate_constraint`.
    // [spec:pgorm:req:bindings.schema-domains+2]
    #[pyo3(signature=(condition, *, name=None, not_valid=false))]
    fn add_check(
        &self,
        condition: &Bound<'_, PyAny>,
        name: Option<&Bound<'_, PyAny>>,
        not_valid: bool,
    ) -> PyResult<PyDDL> {
        let check = check(condition, name)?;
        Ok(self.altered(|pending| match not_valid {
            true => pending.add_check(check.not_valid()),
            false => pending.add_check(check),
        }))
    }

    // [spec:pgorm:req:bindings.schema-domains+2]
    #[pyo3(signature=(name, *, if_exists=false, behavior=None))]
    fn drop_constraint(
        &self,
        name: &Bound<'_, PyAny>,
        if_exists: bool,
        behavior: Option<&str>,
    ) -> PyResult<PyDDL> {
        let mut drop = ConstraintDrop::new(args::name(name)?);
        if if_exists {
            drop = drop.if_exists();
        }
        drop = match args::behavior(behavior)? {
            Some(DropBehavior::Cascade) => drop.cascade(),
            Some(DropBehavior::Restrict) => drop.restrict(),
            None => drop,
        };
        Ok(self.altered(|pending| pending.drop_constraint(drop)))
    }

    fn rename_constraint(
        &self,
        name: &Bound<'_, PyAny>,
        new_name: &Bound<'_, PyAny>,
    ) -> PyResult<PyDDL> {
        let (name, new_name) = (args::name(name)?, args::name(new_name)?);
        Ok(self.altered(|pending| pending.rename_constraint(name, new_name)))
    }

    /// Checks the values a `NOT VALID` check left unchecked.
    fn validate_constraint(&self, name: &Bound<'_, PyAny>) -> PyResult<PyDDL> {
        let name = args::name(name)?;
        Ok(self.altered(|pending| pending.validate_constraint(name)))
    }

    /// The domain renamed; the new name is bare, the domain staying in its
    /// schema.
    // [spec:pgorm:req:bindings.schema-domains+2]
    fn rename_to(&self, name: &Bound<'_, PyAny>) -> PyResult<PyDDL> {
        let name = args::name(name)?;
        Ok(self.altered(|pending| pending.rename_to(name)))
    }

    // [spec:pgorm:req:python.truth-testing+4]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "an ALTER DOMAIN with no action cannot be tested as a Python boolean",
        ))
    }
}

/// `alter_domain(name)`: the domain, with no action yet.
// [spec:pgorm:req:bindings.schema-domains+2]
#[pyfunction]
pub(super) fn alter_domain(name: &Bound<'_, PyAny>) -> PyResult<PyPendingAlterDomain> {
    Ok(PyPendingAlterDomain {
        pending: Domain::alter(type_ref(name)?),
    })
}

/// `DROP DOMAIN` over one domain or a non-empty list of them, in order.
// [spec:pgorm:req:bindings.schema-domains+2]
#[pyfunction]
#[pyo3(signature=(names, *, if_exists=false, behavior=None))]
pub(super) fn drop_domain(
    names: &Bound<'_, PyAny>,
    if_exists: bool,
    behavior: Option<&str>,
) -> PyResult<PyDDL> {
    let drop = drop_list(type_refs(names)?, if_exists, behavior)?;
    Ok(PyDDL::new(Statement::DropDomain(drop)))
}
