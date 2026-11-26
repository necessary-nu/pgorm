//! The compile errors a write on a connection that may only read is.
//!
//! Each fixture in `tests/read-only-fail/` is paired with the `.stderr`
//! trybuild holds it to. The messages are pgorm's own, written through
//! `#[diagnostic::on_unimplemented]` on the trait each write needs, so a
//! change to their wording shows here.

// [spec:pgorm:req:read-only.diagnostics/test]    an ORM write, text, a `sql!` write, a raw selector over text and COPY FROM, each on a connection that may only read — a read-only connection, a read-only transaction's handle, a connection's `read_only()` and code generic over `Connection` — fail with the trait's own message naming the connection
// [spec:pgorm:def:read-only.traits/test]    every connection that may only read lacks `ConnectionMut`, whose writes and `batch_execute` are compile errors on it
// [spec:pgorm:req:read-only.writes/test]    an insert, an update and a delete are refused on every connection that may only read
// [spec:pgorm:req:read-only.text/test]    text not known to read, and a raw selector over it, run on no connection that may only read
#[test]
fn read_only_compile_fail_tests() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/read-only-fail/*.rs");
}
