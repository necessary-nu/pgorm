//! `Expr`'s `jsonb` accessors and key tests: `->`, `->>`, `#>`, `#>>`, `?`,
//! `?|` and `?&`, through pgorm-query's methods of those meanings.

use pgorm::pgorm_query::Expr;
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyList, PyString, PyTuple},
};

use super::{PyExpr, coerce};

fn text(value: &Bound<'_, PyAny>, what: &str) -> PyResult<String> {
    value
        .cast::<PyString>()
        .map_err(|_| PyTypeError::new_err(format!("{what} is a str")))?
        .to_str()
        .map(str::to_owned)
}

fn texts(values: &Bound<'_, PyAny>, what: &str) -> PyResult<Vec<String>> {
    if !values.is_exact_instance_of::<PyList>() && !values.is_exact_instance_of::<PyTuple>() {
        return Err(PyTypeError::new_err(format!(
            "{what} is a list or tuple of str"
        )));
    }
    values
        .try_iter()?
        .map(|value| text(&value?, what))
        .collect()
}

// [spec:pgorm:req:bindings.json-operators]
#[pymethods]
impl PyExpr {
    /// `->`: the field `key` names, or the array element an integer index
    /// does, as `jsonb`. The key is bound unless it is an expression.
    fn get_json_field(&self, key: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_rust(
            Expr::expr(self.inner.clone()).get_json_field(coerce(key)?.inner),
        ))
    }
    /// `->>`: the field or element as `text`.
    fn cast_json_field(&self, key: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_rust(
            Expr::expr(self.inner.clone()).cast_json_field(coerce(key)?.inner),
        ))
    }
    /// `#>`: the value at `path`, a list of keys bound as one `text[]`.
    fn get_json_path(&self, path: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_rust(
            Expr::expr(self.inner.clone()).get_json_path(texts(path, "a JSON path")?),
        ))
    }
    /// `#>>`: the value at `path` as `text`.
    fn cast_json_path(&self, path: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_rust(
            Expr::expr(self.inner.clone()).cast_json_path(texts(path, "a JSON path")?),
        ))
    }
    /// `?`: whether `key` is a top-level key, or a string element.
    fn has_json_key(&self, key: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_rust(
            Expr::expr(self.inner.clone()).has_json_key(text(key, "a JSON key")?),
        ))
    }
    /// `?|`: whether any of `keys` is a top-level key or a string element.
    fn has_any_json_keys(&self, keys: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_rust(
            Expr::expr(self.inner.clone()).has_any_json_keys(texts(keys, "JSON keys")?),
        ))
    }
    /// `?&`: whether every one of `keys` is.
    fn has_all_json_keys(&self, keys: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_rust(
            Expr::expr(self.inner.clone()).has_all_json_keys(texts(keys, "JSON keys")?),
        ))
    }
}
