//! The `sql!` half: hold a literal's text to the grammar and report refusals.

use syn::{Error, LitStr};

/// Hold the literal's text to the grammar, a `;`-separated script statement
/// by statement.
///
/// The span is the whole literal: pointing at the offending byte inside it would
/// need `proc_macro::Literal::subspan`, which is nightly-only, so the position
/// rides in the message as a caret under the offending line instead.
// [spec:pgorm:req:macros.sql.reject]    a grammar rejection, spanned on the literal
// [spec:pgorm:req:macros.sql.ceiling]    the span is the whole literal
pub(crate) fn validate(literal: &LitStr) -> Result<(), Error> {
    pgorm_sql_check::check_sql(&literal.value())
        .map_err(|rejection| Error::new(literal.span(), rejection))
}
