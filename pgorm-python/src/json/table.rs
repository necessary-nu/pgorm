//! `JSON_TABLE`, a FROM item reading rows out of a JSON document, over
//! `pgorm_query`'s builder. Its paths are Python `str`s the Rust renderer
//! writes as escaped literals, because PostgreSQL refuses a parameter there;
//! the PASSING values are bound.

use pgorm::pgorm_query::{Func, JsonTableBehavior, JsonTableColumn};
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyDict, PyTuple},
};

use super::{
    input,
    kinds::{PyJsonExistsBehavior, query_behavior, value_behavior},
    variables,
};
use crate::{
    errors::ConstructionError, identifiers::Identifier, schema::PyDataType, statements::PyFromItem,
};

/// What `JSON_TABLE` produces when its root path fails: `ON ERROR`. Without
/// one it produces no rows, as `Empty` says; PostgreSQL takes no other.
#[pyclass(name = "JsonTableBehavior", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PyJsonTableBehavior {
    Error,
    Empty,
}

impl PyJsonTableBehavior {
    pub(super) fn rust(&self) -> JsonTableBehavior {
        match self {
            Self::Error => JsonTableBehavior::Error,
            Self::Empty => JsonTableBehavior::Empty,
        }
    }
}

#[pymethods]
impl PyJsonTableBehavior {
    // [spec:pgorm:req:python.truth-testing+4]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL/JSON clauses cannot be tested as Python booleans",
        ))
    }
}

// [spec:pgorm:req:bindings.sql-json]
/// One column of a `JSON_TABLE`, built by one of five static constructors,
/// each taking only the clauses PostgreSQL admits for that kind of column.
#[pyclass(name = "JsonTableColumn", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyJsonTableColumn {
    inner: JsonTableColumn,
}

/// A first column and the rest, as the Rust builders take them.
fn columns(
    first: PyRef<'_, PyJsonTableColumn>,
    rest: &Bound<'_, PyTuple>,
) -> PyResult<(JsonTableColumn, Vec<JsonTableColumn>)> {
    let rest = rest
        .iter()
        .map(|column| {
            column
                .extract::<PyRef<'_, PyJsonTableColumn>>()
                .map(|column| column.inner.clone())
                .map_err(|_| PyTypeError::new_err("JSON_TABLE's columns require JsonTableColumn"))
        })
        .collect::<PyResult<_>>()?;
    Ok((first.inner.clone(), rest))
}

#[pymethods]
impl PyJsonTableColumn {
    /// `name FOR ORDINALITY`: the row's number, counting from 1.
    #[staticmethod]
    fn ordinality(name: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: JsonTableColumn::ordinality(Identifier::new(name)?.name()),
        })
    }

    /// `name kind`: the scalar its path finds, read as `JSON_VALUE` reads it.
    #[staticmethod]
    #[pyo3(signature = (name, kind, *, path=None, on_empty=None, on_error=None))]
    fn value(
        name: &Bound<'_, PyAny>,
        kind: &Bound<'_, PyAny>,
        path: Option<String>,
        on_empty: Option<&Bound<'_, PyAny>>,
        on_error: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let mut column = JsonTableColumn::value(
            Identifier::new(name)?.name(),
            PyDataType::coerce(kind)?.inner,
        );
        if let Some(path) = path {
            column = column.path(path);
        }
        if let Some(behavior) = on_empty {
            column = column.on_empty(value_behavior(behavior)?);
        }
        if let Some(behavior) = on_error {
            column = column.on_error(value_behavior(behavior)?);
        }
        Ok(Self {
            inner: column.into(),
        })
    }

    /// `name kind FORMAT JSON`: the JSON its path finds, read as `JSON_QUERY`
    /// reads it.
    #[staticmethod]
    #[pyo3(signature = (name, kind, *, path=None, shaping=None, on_empty=None, on_error=None))]
    fn query(
        name: &Bound<'_, PyAny>,
        kind: &Bound<'_, PyAny>,
        path: Option<String>,
        shaping: Option<&str>,
        on_empty: Option<&Bound<'_, PyAny>>,
        on_error: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let mut column = JsonTableColumn::query(
            Identifier::new(name)?.name(),
            PyDataType::coerce(kind)?.inner,
        );
        if let Some(path) = path {
            column = column.path(path);
        }
        column = match shaping {
            None => column,
            Some("with_wrapper") => column.with_wrapper(),
            Some("with_conditional_wrapper") => column.with_conditional_wrapper(),
            Some("omit_quotes") => column.omit_quotes(),
            Some(_) => {
                return Err(PyTypeError::new_err(
                    "shaping requires 'with_wrapper', 'with_conditional_wrapper' or 'omit_quotes'",
                ));
            }
        };
        if let Some(behavior) = on_empty {
            column = column.on_empty(query_behavior(behavior)?);
        }
        if let Some(behavior) = on_error {
            column = column.on_error(query_behavior(behavior)?);
        }
        Ok(Self {
            inner: column.into(),
        })
    }

    /// `name kind EXISTS`: whether its path finds anything. There is no
    /// `on_empty`: finding nothing is false.
    #[staticmethod]
    #[pyo3(signature = (name, kind, *, path=None, on_error=None))]
    fn exists(
        name: &Bound<'_, PyAny>,
        kind: &Bound<'_, PyAny>,
        path: Option<String>,
        on_error: Option<PyJsonExistsBehavior>,
    ) -> PyResult<Self> {
        let mut column = JsonTableColumn::exists(
            Identifier::new(name)?.name(),
            PyDataType::coerce(kind)?.inner,
        );
        if let Some(path) = path {
            column = column.path(path);
        }
        if let Some(behavior) = on_error {
            column = column.on_error(behavior.rust());
        }
        Ok(Self {
            inner: column.into(),
        })
    }

    /// `NESTED PATH path COLUMNS (..)`: a row per item `path` finds under the
    /// parent row's, joined to it as an outer join would be. It takes its
    /// first column, as `json_table` does, since an empty list is refused.
    #[staticmethod]
    #[pyo3(signature = (path, column, *columns, path_name=None))]
    fn nested(
        path: String,
        column: PyRef<'_, Self>,
        columns: &Bound<'_, PyTuple>,
        path_name: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let (first, rest) = self::columns(column, columns)?;
        let mut nested = rest
            .into_iter()
            .fold(JsonTableColumn::nested(path, first), |nested, column| {
                nested.column(column)
            });
        if let Some(name) = path_name {
            nested = nested.path_name(Identifier::new(name)?.name());
        }
        Ok(Self {
            inner: nested.into(),
        })
    }

    fn __repr__(&self) -> &'static str {
        "JsonTableColumn(...)"
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "JSON_TABLE columns cannot be tested as Python booleans",
        ))
    }
}

// [spec:pgorm:req:bindings.sql-json]
/// `JSON_TABLE(context, path COLUMNS (..)) AS alias`, a FROM item. The first
/// column and the alias are required, because PostgreSQL refuses an empty
/// column list and pgorm names every FROM item that is not a table.
#[pyfunction]
#[pyo3(signature = (context, path, column, *columns, alias, passing=None, path_name=None, on_error=None))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn json_table(
    context: &Bound<'_, PyAny>,
    path: String,
    column: PyRef<'_, PyJsonTableColumn>,
    columns: &Bound<'_, PyTuple>,
    alias: &Bound<'_, PyAny>,
    passing: Option<&Bound<'_, PyDict>>,
    path_name: Option<&Bound<'_, PyAny>>,
    on_error: Option<PyJsonTableBehavior>,
) -> PyResult<PyFromItem> {
    let (first, rest) = self::columns(column, columns)?;
    let mut table = rest.into_iter().fold(
        Func::json_table(input(context)?, path, first),
        |table, column| table.column(column),
    );
    for (value, name) in variables(passing)? {
        table = table.passing(value, name);
    }
    if let Some(name) = path_name {
        table = table.path_name(Identifier::new(name)?.name());
    }
    if let Some(behavior) = on_error {
        table = table.on_error(behavior.rust());
    }
    Ok(PyFromItem {
        inner: table.alias(Identifier::new(alias)?.name()),
    })
}
