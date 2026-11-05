//! `Expr`'s text matching: the literal substring helpers, LIKE, ILIKE and
//! `SIMILAR TO` over a pattern or a `LikeExpr`, and the POSIX regular-expression
//! operators.

use pgorm::pgorm_query::{BinOper, Expr, Func, Name, SimpleExpr};
use pyo3::prelude::*;

use super::{PyExpr, coerce, options};
use crate::statements::PyQuantified;

impl PyExpr {
    /// `operator` over `pattern` when it is `ANY (..)` or `ALL (..)`, which
    /// carries no escape: PostgreSQL reads a quantifier after LIKE and ILIKE
    /// and their negations, with no `ESCAPE` after it, and refuses one after
    /// `SIMILAR TO`, which `binary` refuses in turn.
    fn quantified(&self, operator: BinOper, pattern: &Bound<'_, PyAny>) -> Option<PyResult<Self>> {
        pattern
            .is_instance_of::<PyQuantified>()
            .then(|| self.binary(operator, pattern))
    }
}

// [spec:pgorm:req:bindings.expressions+3]
// [spec:pgorm:req:bindings.quantifiers+3]
#[pymethods]
impl PyExpr {
    fn starts_with(&self, text: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_rust(
            Func::starts_with(self.inner.clone(), coerce(text)?.inner).into(),
        ))
    }
    fn contains_text(&self, text: &Bound<'_, PyAny>) -> PyResult<Self> {
        let position =
            Func::named(Name::runtime("strpos")).args([self.inner.clone(), coerce(text)?.inner]);
        Ok(Self::from_rust(
            Expr::expr(position).gt(SimpleExpr::Constant(0i32.into())),
        ))
    }
    fn ends_with(&self, text: &Bound<'_, PyAny>) -> PyResult<Self> {
        let text = coerce(text)?.inner;
        let suffix = Func::named(Name::runtime("right"))
            .args([self.inner.clone(), Func::char_length(text.clone()).into()]);
        Ok(Self::from_rust(Expr::expr(suffix).eq(text)))
    }
    fn like(&self, pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Some(quantified) = self.quantified(BinOper::Like, pattern) {
            return quantified;
        }
        Ok(Self::from_rust(
            self.inner.clone().like(options::pattern(pattern)?),
        ))
    }
    fn not_like(&self, pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Some(quantified) = self.quantified(BinOper::NotLike, pattern) {
            return quantified;
        }
        Ok(Self::from_rust(
            self.inner.clone().not_like(options::pattern(pattern)?),
        ))
    }
    fn ilike(&self, pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Some(quantified) = self.quantified(BinOper::ILike, pattern) {
            return quantified;
        }
        Ok(Self::from_rust(
            Expr::expr(self.inner.clone()).ilike(options::pattern(pattern)?),
        ))
    }
    fn not_ilike(&self, pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Some(quantified) = self.quantified(BinOper::NotILike, pattern) {
            return quantified;
        }
        Ok(Self::from_rust(
            Expr::expr(self.inner.clone()).not_ilike(options::pattern(pattern)?),
        ))
    }
    /// `SIMILAR TO`: a whole-string match in SQL's regular-expression
    /// dialect, the pattern bound and its ESCAPE carried as LIKE's are.
    fn similar_to(&self, pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Some(quantified) = self.quantified(BinOper::SimilarTo, pattern) {
            return quantified;
        }
        Ok(Self::from_rust(
            self.inner.clone().similar_to(options::pattern(pattern)?),
        ))
    }
    fn not_similar_to(&self, pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Some(quantified) = self.quantified(BinOper::NotSimilarTo, pattern) {
            return quantified;
        }
        Ok(Self::from_rust(
            self.inner
                .clone()
                .not_similar_to(options::pattern(pattern)?),
        ))
    }

    /// `~`: whether the text matches a POSIX regular expression, case
    /// sensitively. Unlike LIKE's, the pattern is an operand, bound unless
    /// it is an expression.
    fn regex(&self, pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(BinOper::Regex, pattern)
    }
    fn not_regex(&self, pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(BinOper::NotRegex, pattern)
    }
    fn iregex(&self, pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(BinOper::RegexCaseInsensitive, pattern)
    }
    fn not_iregex(&self, pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(BinOper::NotRegexCaseInsensitive, pattern)
    }
}
