//! Every binary operator pgorm-query names against the Python that writes
//! it: an operator no expression method writes is one named here as left
//! out, so one pgorm-query gains fails this test until it is either.

use std::ffi::CString;

use pgorm::pgorm_query::{BinOper, SimpleExpr, tests_cfg::Variants};
use pyo3::{
    prelude::*,
    types::{PyDict, PyModule},
};

use super::PyExpr;

/// Python writing one binary operator each, over the columns `a` and `b`.
const SPELLED: [&str; 44] = [
    "a & b",
    "a | b",
    "a.like('x')",
    "a.not_like('x')",
    "a.similar_to('x')",
    "a.not_similar_to('x')",
    "a.is_null()",
    "a.is_not_null()",
    "a.is_distinct_from(1)",
    "a.is_not_distinct_from(1)",
    "a.is_in([1])",
    "a.is_not_in([1])",
    "a.between(1, 2)",
    "a.not_between(1, 2)",
    "a.between(1, 2, symmetric=True)",
    "a.not_between(1, 2, symmetric=True)",
    "a == 1",
    "a != 1",
    "a < 1",
    "a > 1",
    "a <= 1",
    "a >= 1",
    "a + 1",
    "a - 1",
    "a * 1",
    "a / 1",
    "a % 1",
    "a.strictly_left_of(b)",
    "a.strictly_right_of(b)",
    "a.ilike('x')",
    "a.not_ilike('x')",
    "a.matches(b)",
    "a.contains(b)",
    "a.contained_by(b)",
    "a.concat(b)",
    "a.overlaps(b)",
    "a.path_exists('$.x')",
    "a.regex('x')",
    "a.iregex('x')",
    "a.not_regex('x')",
    "a.not_iregex('x')",
    "a.not_extend_right_of(b)",
    "a.not_extend_left_of(b)",
    "a.adjacent_to(b)",
];

// [spec:pgorm:req:bindings.expressions+4/test]
#[test]
fn every_binary_operator_is_written_or_left_out() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "pgorm")?;
        crate::values::register(&module)?;
        crate::identifiers::register(&module)?;
        super::register(&module)?;
        let globals = PyDict::new(py);
        globals.set_item("p", module)?;
        py.run(c"a = p.col('a'); b = p.col('b')", Some(&globals), None)?;
        let mut written = Vec::new();
        for source in SPELLED {
            let expr = py.eval(&CString::new(source)?, Some(&globals), None)?;
            match &expr.extract::<PyRef<'_, PyExpr>>()?.inner {
                SimpleExpr::Binary(_, operator, _) => written.push(*operator),
                other => panic!("{source} writes {other:?}, no binary operator"),
            }
        }
        // `As` is a projection's alias, no operator a program applies; the
        // rest are pgorm-query's pg_trgm, JSON, time zone and pgvector
        // operators and its raw one, which the module does not spell.
        assert_eq!(
            BinOper::missing_from(&written),
            [
                "As",
                "Similarity",
                "WordSimilarity",
                "StrictWordSimilarity",
                "SimilarityDistance",
                "WordSimilarityDistance",
                "StrictWordSimilarityDistance",
                "GetJsonField",
                "CastJsonField",
                "GetJsonPath",
                "CastJsonPath",
                "HasJsonKey",
                "HasAnyJsonKeys",
                "HasAllJsonKeys",
                "AtTimeZone",
                "EuclideanDistance",
                "NegativeInnerProduct",
                "CosineDistance",
                "Raw",
            ]
        );
        Ok(())
    })
}
