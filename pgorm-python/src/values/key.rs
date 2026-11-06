//! A kind spelled as one string, as a model's and a registered entity's
//! `describe()` report it: `i64`, `text[]`, `enum "app"."mood"`,
//! `range "app"."floatrange" of f64`.

use pyo3::prelude::*;

use super::PyValue;

#[pymethods]
impl PyValue {
    /// The value's kind as a description spells it.
    fn _kind_key(&self) -> String {
        self.tag.key()
    }
}
