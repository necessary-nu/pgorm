//! Compile-time verification of both macros' contracts.
//!
//! The `compile-fail` fixtures are the refusals, each paired with a `.stderr`
//! snapshot trybuild asserts against: for `sql!`, malformed grammar and inputs
//! that are not a lone string literal; for `prql!`, PRQL the compiler rejects
//! (including `take $N`, and a whole-row grouping that selects the star it
//! groups by), emitted SQL the oracle rejects (the broken
//! s-string), and every placeholder mistake — too few arguments, too many,
//! a gap in the numbering, and `$0`. The `pass` fixtures are compiled *and
//! run*, so the expansions are checked live and their assertions are part of
//! this test.

// [spec:pgorm:req:macros.sql.reject/test]    grammar rejections and non-literal input
// [spec:pgorm:def:macros.sql+6/test]    valid SQL compiles and keeps its text
// [spec:pgorm:req:macros.sql.ceiling/test]    unknown tables pass; the span is the whole literal
// [spec:pgorm:req:macros.prql.reject/test]    all five refusals, spanned and named
// [spec:pgorm:sem:macros.prql.sstring/test]    a broken s-string dies at the oracle
// [spec:pgorm:def:macros.prql/test]    valid PRQL expands to SQL plus Values
#[test]
fn compile_fail_tests() {
    let t = trybuild::TestCases::new();
    t.compile_fail("./tests/compile-fail/*.rs");
    t.pass("./tests/pass/*.rs");
}
