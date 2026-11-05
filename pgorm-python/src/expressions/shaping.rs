//! What turns an `Expr` into something else: a cast, a collation, an alias,
//! a window call, a sort key, and its inspection.

use pgorm::pgorm_query::{Expr, ExprTrait, Query};
use pyo3::prelude::*;

use super::{Aliased, Compiled, OrderBy, PyExpr};
use crate::{
    identifiers::{Direction, Identifier, Nulls, collation},
    values::PyTypeName,
};

// [spec:pgorm:req:bindings.expressions+4]
#[pymethods]
impl PyExpr {
    #[pyo3(signature = (type_name, *, array=false))]
    fn cast(&self, type_name: PyRef<'_, PyTypeName>, array: bool) -> Self {
        let name = type_name.rust_type();
        Self::from_rust(
            self.inner
                .clone()
                .cast_as_type(if array { name.array() } else { name }),
        )
    }
    /// `(expr COLLATE "name")`: this expression under a collation, through
    /// Rust's `ExprTrait::collate`. It stays an operand, so it goes on to be
    /// compared, and `asc` or `desc` over it writes the clause into the sort
    /// key, where PostgreSQL reads an `ORDER BY`'s collation.
    // [spec:pgorm:req:bindings.collations]
    #[pyo3(signature = (name, *, schema=None))]
    fn collate(
        &self,
        name: &Bound<'_, PyAny>,
        schema: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let collated = Expr::expr(self.inner.clone()).collate(collation(name, schema)?);
        Ok(Self::from_rust(collated))
    }
    fn as_(&self, alias: &Bound<'_, PyAny>) -> PyResult<Aliased> {
        Ok(Aliased {
            expr: self.clone(),
            alias: Identifier::new(alias)?,
        })
    }
    /// This call over a window: a `Window` or an `ExtendedWindow` written
    /// inline, or the name of the one a statement declares with
    /// `Select.window`. Only a function call or a JSON aggregate takes one,
    /// as PostgreSQL's grammar has it.
    fn over(&self, window: &Bound<'_, PyAny>) -> PyResult<crate::statements::PyWindowed> {
        let call = crate::statements::WindowCall::of(&self.inner)?;
        call.over(window)
    }
    #[pyo3(signature = (*, nulls=None))]
    fn asc(&self, nulls: Option<Nulls>) -> OrderBy {
        OrderBy {
            expr: self.clone(),
            direction: Direction::Asc,
            nulls,
        }
    }
    #[pyo3(signature = (*, nulls=None))]
    fn desc(&self, nulls: Option<Nulls>) -> OrderBy {
        OrderBy {
            expr: self.clone(),
            direction: Direction::Desc,
            nulls,
        }
    }

    /// Inspect this expression as the projection of a Rust-built SELECT.
    fn inspect(&self) -> Compiled {
        let (sql, values) = Query::select().expr(self.inner.clone()).build();
        Compiled { sql, values }
    }
    fn __repr__(&self) -> &'static str {
        "Expr(...)"
    }
}
