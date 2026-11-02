// The `pass` / `pass-static` fixtures are compiled *and run*, so their
// assertions are part of this test: variant `#[iden = "..."]` and
// `#[iden(rename = "...")]`, `#[method = "..."]` and `#[iden(method = "...")]`,
// `#[iden(flatten)]` (including nested), the `Table` variant rendering the
// container name, and `prepare` being emitted only for statically-valid idens.
// The `compile-fail` fixtures pin the refusals: non-enum / non-unit-struct
// containers, container-level list forms, bare `#[iden]` / `#[method]` paths,
// non-string literals, and multi-field flatten.
// [spec:pgorm:sem:macros.derive.iden.query+1/test]
#[test]
fn build_tests() {
    let t = trybuild::TestCases::new();
    t.compile_fail("./tests/compile-fail/*.rs");

    // all of these are exactly the same as the examples in `examples/derive.rs`
    t.pass("./tests/pass/*.rs");
    t.pass("./tests/pass-static/*.rs");
}
