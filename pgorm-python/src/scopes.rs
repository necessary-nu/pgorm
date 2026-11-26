//! A Python context and a declared model's scope columns, as pgorm-bindings
//! decides what each holds a statement to (`bindings.scopes`). The package
//! reads a program's `Scope`, `Every` or `ScopeContext` into the items a
//! `_ScopeContext` is made from; what a scope does with them is
//! pgorm-bindings' own.

use pgorm::pgorm_query::Value;
use pgorm_bindings::{
    entity::EntityInfo,
    scope::{self, Context, DeclaredScopes, Holding, RegisteredScope, ScopeColumn, Scoping},
};
use pyo3::{prelude::*, types::PyMapping};
use std::sync::Arc;

use crate::{
    config::DIALECT,
    entities::input,
    errors::{ConstructionError, Raise},
    expressions::PyExpr,
    statements::conflict_clause,
    values::PyValue,
};

/// A program's context, its scopes by name: each one value, or every value.
// [spec:pgorm:req:bindings.scopes]
#[pyclass(name = "_ScopeContext", module = "pgorm", frozen)]
pub struct PyScopeContext {
    pub(crate) inner: Context<Py<PyAny>>,
}

#[pymethods]
impl PyScopeContext {
    /// The context `label` names, holding each of `held`: a scope's name,
    /// and its value, or `None` with `every` for every value of it.
    #[new]
    fn new(label: String, held: Vec<(String, Py<PyAny>, bool)>) -> PyResult<Self> {
        let held = held
            .into_iter()
            .map(|(name, value, every)| {
                (
                    name,
                    match every {
                        true => Holding::Every,
                        false => Holding::One(value),
                    },
                )
            })
            .collect();
        Ok(Self {
            inner: Context::new(label, held).py()?,
        })
    }

    #[getter]
    fn label(&self) -> &str {
        self.inner.label()
    }
}

impl PyScopeContext {
    /// `value`, a program's context, read by the package's `scopes.held`:
    /// a scope value, an `Every` or a `ScopeContext`.
    pub(crate) fn read<'py>(value: &Bound<'py, PyAny>) -> PyResult<Bound<'py, Self>> {
        if let Ok(cx) = value.cast::<Self>() {
            return Ok(cx.clone());
        }
        Ok(value
            .py()
            .import("pgorm.scopes")?
            .getattr("held")?
            .call1((value,))?
            .cast_into::<Self>()?)
    }

    /// The context's values as the scopes `info`'s registry knows take
    /// them, `None` for a statement given no context.
    pub(crate) fn values(cx: Option<&Self>, info: &EntityInfo) -> PyResult<Option<Context<Value>>> {
        cx.map(|cx| Python::attach(|py| registered(py, &cx.inner, &info.scopes.get())))
            .transpose()
    }

    /// The Rust scope values a registered statement on `info`'s entity runs
    /// under: `None` is a statement given no context, refused where the
    /// entity is scoped, as is a context not holding its scopes.
    // [spec:pgorm:req:bindings.scopes-registered]
    pub(crate) fn scoping(cx: Option<&Self>, info: &EntityInfo) -> PyResult<Arc<Scoping>> {
        info.scoping(Self::values(cx, info)?.as_ref(), DIALECT).py()
    }
}

/// `cx`'s values for the scopes `scopes` lists, each converted as the
/// scope's column converts a value: a scope it does not list is no Rust
/// scope a registered statement reads, and is left out.
fn registered(
    py: Python<'_>,
    cx: &Context<Py<PyAny>>,
    scopes: &[Arc<RegisteredScope>],
) -> PyResult<Context<Value>> {
    let mut held = Vec::new();
    for (name, holding) in cx.held() {
        let Some(scope) = scopes.iter().find(|scope| scope.name == name) else {
            continue;
        };
        let holding = match holding {
            Holding::One(value) => {
                Holding::One(input::coerce(&scope.column, value.bind(py))?.inner)
            }
            Holding::Every => Holding::Every,
        };
        held.push((name.to_owned(), holding));
    }
    Context::new(cx.label(), held).py()
}

/// A declared model's scope columns: what a statement on its table under a
/// context is held to, what an insert fills, and what each refuses.
// [spec:pgorm:req:bindings.scopes]
#[pyclass(name = "_ModelScopes", module = "pgorm", frozen)]
pub struct PyModelScopes {
    inner: DeclaredScopes,
}

#[pymethods]
impl PyModelScopes {
    /// The scope columns of `table`, each its field, its column's SQL name
    /// and its scope's name: one field per scope.
    #[new]
    fn new(table: String, columns: Vec<(String, String, String)>) -> PyResult<Self> {
        let columns = columns
            .into_iter()
            .map(|(field, column, scope)| ScopeColumn {
                field,
                column,
                scope,
            })
            .collect();
        Ok(Self {
            inner: DeclaredScopes::new(table, columns).py()?,
        })
    }

    /// The scopes, in declaration order.
    #[getter]
    fn names(&self) -> Vec<String> {
        self.inner.names().into_iter().map(str::to_owned).collect()
    }

    /// Refuse a statement on the table under `cx`, or under none, unless it
    /// holds each scope.
    fn check(&self, cx: Option<PyRef<'_, PyScopeContext>>) -> PyResult<()> {
        self.inner
            .held(cx.as_deref().map(|cx| &cx.inner), DIALECT)
            .map(drop)
            .py()
    }

    /// Each field a statement under `cx` is held to equal, with the value:
    /// a scope held as every value adds none.
    fn condition(
        &self,
        py: Python<'_>,
        cx: Option<PyRef<'_, PyScopeContext>>,
    ) -> PyResult<Vec<(String, Py<PyAny>)>> {
        Ok(self
            .inner
            .condition(cx.as_deref().map(|cx| &cx.inner), DIALECT)
            .py()?
            .into_iter()
            .map(|(field, value)| (field.to_owned(), value.clone_ref(py)))
            .collect())
    }

    /// The fields an insert under `cx` fills in `row`, a mapping of field to
    /// value, each with the context's value; `convert(field, value)` makes a
    /// value of either side the field's `Value`. A row setting a scope
    /// column to an expression is outside the scope, as no value it holds
    /// can be compared.
    fn fill(
        &self,
        py: Python<'_>,
        cx: Option<PyRef<'_, PyScopeContext>>,
        row: &Bound<'_, PyMapping>,
        convert: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<(String, Py<PyAny>)>> {
        let set =
            |field: &str| -> Option<Py<PyAny>> { row.get_item(field).ok().map(Bound::unbind) };
        let compared = |column: &ScopeColumn, value: &Py<PyAny>| -> PyResult<Option<Value>> {
            let value = value.bind(py);
            if value.is_instance_of::<PyExpr>() {
                return Ok(None);
            }
            let converted = convert.call1((&column.field, value))?;
            Ok(Some(
                converted.extract::<PyRef<'_, PyValue>>()?.inner.clone(),
            ))
        };
        let fills = self
            .inner
            .fill(cx.as_deref().map(|cx| &cx.inner), set, compared, DIALECT)?
            .py()?;
        fills
            .into_iter()
            .map(|(field, value)| {
                let value = value.ok_or_else(|| {
                    ConstructionError::new_err("a context's scope value is no expression")
                })?;
                Ok((
                    field.to_owned(),
                    Py::new(py, PyValue::from_rust(value))?.into_any(),
                ))
            })
            .collect()
    }

    /// Refuse an update setting any of `fields` that is a scope column.
    fn immutable(&self, fields: Vec<String>) -> PyResult<()> {
        self.inner.immutable(fields.iter().map(String::as_str)).py()
    }

    /// Refuse an insert's conflict action `action` where its `DO UPDATE
    /// SET` assigns a scope column.
    fn upsert(&self, action: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.upsert(&conflict_clause(action)?).py()
    }

    /// Refuse a join into the table by a form taking no context, where the
    /// table is scoped.
    fn join(&self) -> PyResult<()> {
        match self.inner.columns().is_empty() {
            true => Ok(()),
            false => Err(scope::unscoped_join(
                self.inner.table(),
                &self.inner.names(),
                DIALECT,
            ))
            .py(),
        }
    }
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyScopeContext>()?;
    module.add_class::<PyModelScopes>()?;
    Ok(())
}
