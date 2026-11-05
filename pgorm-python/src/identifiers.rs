use pgorm::pgorm_query::{Collation, IntoCollation, Name, NullOrdering, Order};
use pyo3::{exceptions::PyTypeError, prelude::*};

use crate::errors::ConstructionError;

// [spec:pgorm:req:bindings.identifiers+3]
/// One identifier as an identifier position reads it: a `str`, validated,
/// which `name` mints with Rust's `Name::runtime`, so its punctuation is never
/// parsed as SQL. Python has no class for it: every name a binding is given is
/// computed at run time, so the position that takes the text is where Rust's
/// one route from text to a name runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identifier {
    pub name: String,
}

impl Identifier {
    pub fn new(name: &Bound<'_, PyAny>) -> PyResult<Self> {
        let name = name
            .extract::<String>()
            .map_err(|_| PyTypeError::new_err("identifier requires a string"))?;
        validate_name(&name)?;
        Ok(Self { name })
    }

    pub fn name(&self) -> Name {
        Name::runtime(&self.name)
    }
}

/// The identifier `name` holds, refused as every identifier position refuses
/// one: what a model declaration checks its names with before any statement
/// reads them.
// [spec:pgorm:req:bindings.identifiers+3]
#[pyfunction(name = "_identifier")]
fn identifier(name: &Bound<'_, PyAny>) -> PyResult<String> {
    Identifier::new(name).map(|identifier| identifier.name)
}

pub(crate) fn validate_name(name: &str) -> PyResult<()> {
    if name.is_empty() || name.contains('\0') || name.len() > 63 {
        Err(ConstructionError::new_err(
            "identifier parts require 1–63 UTF-8 bytes without NUL",
        ))
    } else {
        Ok(())
    }
}

/// A collation named by `name`, qualified by `schema` when one is given, each
/// an identifier: what `Expr.collate`, `ColumnDef.collate` and
/// `CreateDomain.collate` take, so the three refuse the same names.
pub(crate) fn collation(
    name: &Bound<'_, PyAny>,
    schema: Option<&Bound<'_, PyAny>>,
) -> PyResult<Collation> {
    let name = Identifier::new(name)?.name();
    Ok(match schema {
        Some(schema) => (Identifier::new(schema)?.name(), name).into_collation(),
        None => name.into_collation(),
    })
}

/// Typed SQL ordering direction.
#[pyclass(name = "Direction", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Direction {
    Asc,
    Desc,
}

#[pymethods]
impl Direction {
    // [spec:pgorm:req:python.truth-testing+4]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "sort directions cannot be tested as Python booleans",
        ))
    }
}

impl Direction {
    pub fn rust_order(&self) -> Order {
        match self {
            Self::Asc => Order::Asc,
            Self::Desc => Order::Desc,
        }
    }
}

/// Typed placement of NULL values in an ordering.
#[pyclass(name = "Nulls", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Nulls {
    First,
    Last,
}

#[pymethods]
impl Nulls {
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "NULL orderings cannot be tested as Python booleans",
        ))
    }
}

impl Nulls {
    pub fn rust_nulls(&self) -> NullOrdering {
        match self {
            Self::First => NullOrdering::First,
            Self::Last => NullOrdering::Last,
        }
    }
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(identifier, module)?)?;
    module.add_class::<Direction>()?;
    module.add_class::<Nulls>()?;
    Ok(())
}
