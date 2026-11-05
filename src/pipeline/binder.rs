//! The parameter binder: placeholders and values are minted as one.

use std::marker::PhantomData;

use pgorm_query::{ArrayType, Value};

use super::adapter;
use super::compare::{Operand, operand};
use super::expr::Expr;

/// Mints `$N` placeholders against the values the pipeline will carry.
///
/// A binder is only ever handed to the closures the `_with` transforms of a
/// [`Pipeline`](super::Pipeline) take, branded with a fresh lifetime per
/// call. [`bind`](Binder::bind) pushes the value and returns the placeholder
/// in one step, so a placeholder without its value — or a value without its
/// placeholder — cannot be constructed, and the brand pins the resulting
/// expression to this pipeline:
///
/// ```compile_fail,E0521
/// use pgorm::pipeline::{Expr, ExprOps, Pipeline};
/// use pgorm::tests_cfg::cake::{self, Column as C};
///
/// let mut smuggled: Option<Expr<'static>> = None;
/// let _ = Pipeline::from(cake::Entity).filter_with(|binder| {
///     let bound = binder.bind(1_i32);
///     smuggled = Some(bound.clone());
///     C::Id.eq(bound)
/// });
/// ```
///
/// The same pipeline without the escape compiles: drop the `smuggled`
/// assignment and the closure is an ordinary bound filter.
///
/// ```
/// use pgorm::pipeline::{ExprOps, Pipeline};
/// use pgorm::tests_cfg::cake::{self, Column as C};
///
/// let (sql, values) = Pipeline::from(cake::Entity)
///     .filter_with(|binder| C::Id.eq(binder.bind(1_i32)))
///     .into_sql()
///     .expect("the pipeline resolves");
/// assert_eq!(sql, "SELECT * FROM cake WHERE id = $1");
/// assert_eq!(values.0.len(), 1);
/// ```
// [spec:pgorm:req:pipeline.params+4]
#[derive(Debug)]
pub struct Binder<'brand> {
    values: &'brand mut Vec<Value>,
    _brand: PhantomData<fn(&'brand ()) -> &'brand ()>,
}

impl<'brand> Binder<'brand> {
    pub(super) fn new(values: &'brand mut Vec<Value>) -> Self {
        Binder {
            values,
            _brand: PhantomData,
        }
    }

    /// Bind `value` and return its `$N` placeholder.
    ///
    /// Placeholders number in bind order across the whole pipeline. prqlc
    /// may prune an expression nothing reads, placeholder and all, so
    /// [`into_sql`](super::Pipeline::into_sql) re-counts what the emitted
    /// SQL still carries and compacts the [`Values`](pgorm_query::Values) to
    /// match: position `N` in the SQL is position `N` in the values, whether
    /// or not every binding survived.
    ///
    /// A built-in range or multirange, or an array of either, compared under
    /// the cast of a range type a schema created, or of its array, is written
    /// through its text by the server, the one form both read.
    // [spec:pgorm:def:sql.value.created-range+8]
    pub fn bind(&mut self, value: impl Into<Value>) -> Expr<'brand> {
        let value = value.into();
        let bound = match (&value, value.source_type_name()) {
            (
                Value::Range(..)
                | Value::Multirange(..)
                | Value::Array(ArrayType::Range(_) | ArrayType::Multirange(_), _),
                Some(pin),
            ) => Operand::BoundRange(pin),
            (_, pin) => Operand::Bound(pin),
        };
        self.values.push(value);
        operand(adapter::param(self.values.len()), bound)
    }
}
