//! A table's column, pgorm-query's `ColumnDef`: its name, type and collation,
//! and the clauses written after them in the order they were added — `NULL`,
//! the one `NOT NULL` constraint, `DEFAULT`, `CHECK`, a generated expression
//! of a stated kind, and an identity with its sequence's options.

use super::{
    args::{self, ENFORCEMENT, choice, pick},
    types::PyDataType,
};
use crate::{
    errors::ConstructionError,
    expressions,
    identifiers::{Identifier, collation},
};
use pgorm::pgorm_query::{Check, ColumnDef, GeneratedKind, IdentityGeneration};
use pyo3::{prelude::*, types::PyDict};

#[derive(Clone, Debug)]
#[pyclass(name = "ColumnDef", module = "pgorm.schema", frozen, from_py_object)]
pub struct PyColumnDef {
    pub(crate) inner: ColumnDef,
}

impl PyColumnDef {
    fn with(&self, change: impl FnOnce(&mut ColumnDef)) -> Self {
        let mut inner = self.inner.clone();
        change(&mut inner);
        Self { inner }
    }

    /// The column a table is created with or a column is added as, which
    /// needs its type.
    pub(super) fn typed(&self, what: &str) -> PyResult<ColumnDef> {
        if self.inner.get_column_type().is_none() {
            return Err(ConstructionError::new_err(format!("{what} needs a type")));
        }
        Ok(self.inner.clone())
    }
}

#[pymethods]
impl PyColumnDef {
    /// A column, its type left out for a `modify_column` that does not retype
    /// it.
    // [spec:pgorm:req:bindings.schema-tables]
    #[new]
    #[pyo3(signature=(name, kind=None))]
    fn new(name: &Bound<'_, PyAny>, kind: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let name = Identifier::new(name)?.name();
        Ok(Self {
            inner: match kind {
                Some(kind) => ColumnDef::new_with_type(name, PyDataType::coerce(kind)?.inner),
                None => ColumnDef::new(name),
            },
        })
    }

    #[getter]
    fn name(&self) -> String {
        self.inner.get_column_name()
    }

    /// `[CONSTRAINT "name" ]NOT NULL[ NO INHERIT]`: the column's one not-null
    /// constraint, named and kept from inheriting tables as the native
    /// `not_null_named` and `not_null_no_inherit` set it.
    // [spec:pgorm:req:bindings.schema-tables]
    #[pyo3(signature=(*, name=None, no_inherit=false))]
    fn not_null(&self, name: Option<&Bound<'_, PyAny>>, no_inherit: bool) -> PyResult<Self> {
        let name = name.map(args::name).transpose()?;
        Ok(self.with(|inner| {
            inner.not_null();
            if let Some(name) = name {
                inner.not_null_named(name);
            }
            if no_inherit {
                inner.not_null_no_inherit();
            }
        }))
    }

    fn null(&self) -> Self {
        let mut inner = self.inner.clone();
        inner.null();
        Self { inner }
    }

    /// The serial family in place of an integer type; `identity` is the form
    /// PostgreSQL recommends.
    fn auto_increment(&self) -> Self {
        let mut inner = self.inner.clone();
        inner.auto_increment();
        Self { inner }
    }

    /// `DEFAULT`: an expression, or a value written as an escaped literal.
    fn default(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let value = expressions::coerce(value)?.inner;
        Ok(self.with(|inner| {
            inner.default(value);
        }))
    }

    // [spec:pgorm:req:bindings.schema-tables]
    #[pyo3(signature=(condition, *, name=None, enforcement=None, no_inherit=false))]
    fn check(
        &self,
        condition: &Bound<'_, PyAny>,
        name: Option<&Bound<'_, PyAny>>,
        enforcement: Option<&str>,
        no_inherit: bool,
    ) -> PyResult<Self> {
        let check = check(condition, name, enforcement, no_inherit)?;
        Ok(self.with(|inner| {
            inner.check(check);
        }))
    }

    /// `COLLATE` after the type: the column's one collation, a later call
    /// replacing an earlier, since PostgreSQL refuses a column with two.
    /// `modify_column` writes it on the retype, the only place PostgreSQL
    /// changes a column's collation.
    // [spec:pgorm:req:bindings.collations]
    #[pyo3(signature=(name, *, schema=None))]
    fn collate(
        &self,
        name: &Bound<'_, PyAny>,
        schema: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let named = collation(name, schema)?;
        Ok(self.with(|inner| {
            inner.collate(named);
        }))
    }

    /// The kind is always named: PostgreSQL 17 refuses a generated column
    /// without one, and 18 reads it as virtual.
    // [spec:pgorm:req:bindings.schema-tables]
    fn generated(&self, expression: &Bound<'_, PyAny>, kind: &str) -> PyResult<Self> {
        let kind = pick(
            kind,
            "a generated column's kind",
            &[
                ("stored", GeneratedKind::Stored),
                ("virtual", GeneratedKind::Virtual),
            ],
        )?;
        let expression = expressions::require_expr(expression)?.inner;
        Ok(self.with(|inner| {
            inner.generated(expression, kind);
        }))
    }

    /// `GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY`, with its sequence's
    /// options when any are given: the options a standalone sequence takes.
    // [spec:pgorm:req:bindings.schema-tables]
    #[pyo3(signature=(generation, **options))]
    fn identity(&self, generation: &str, options: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        let generation = pick(
            generation,
            "an identity's generation",
            &[
                ("always", IdentityGeneration::Always),
                ("by_default", IdentityGeneration::ByDefault),
            ],
        )?;
        let options = args::sequence_options(options)?;
        Ok(self.with(|inner| {
            match options {
                Some(options) => inner.identity_with(generation, options),
                None => match generation {
                    IdentityGeneration::Always => inner.identity(),
                    IdentityGeneration::ByDefault => inner.identity_by_default(),
                },
            };
        }))
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "column definitions cannot be tested as Python booleans",
        ))
    }
}

/// A `CHECK` over `condition`, named, enforced or not and kept from
/// inheriting tables as asked: what a column's, a table's and an added
/// `CHECK` each are.
// [spec:pgorm:req:bindings.schema-tables]
pub(super) fn check(
    condition: &Bound<'_, PyAny>,
    name: Option<&Bound<'_, PyAny>>,
    enforcement: Option<&str>,
    no_inherit: bool,
) -> PyResult<Check> {
    let mut check = Check::new(expressions::require_expr(condition)?.inner);
    if let Some(name) = name {
        check = check.name(args::name(name)?);
    }
    if let Some(enforcement) = choice(enforcement, "an enforcement", ENFORCEMENT)? {
        check = check.enforcement(enforcement);
    }
    if no_inherit {
        check = check.no_inherit();
    }
    Ok(check)
}
