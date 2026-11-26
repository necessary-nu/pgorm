//! The `sql!` half: hold a literal's text to the grammar, report refusals,
//! and type the expansion by what the statement may write.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, LitStr};

/// Hold the literal's text to the grammar, a `;`-separated script statement
/// by statement, and say whether it only reads.
///
/// The span is the whole literal: pointing at the offending byte inside it would
/// need `proc_macro::Literal::subspan`, which is nightly-only, so the position
/// rides in the message as a caret under the offending line instead.
// [spec:pgorm:req:macros.sql.reject+2]    a grammar rejection, spanned on the literal
// [spec:pgorm:req:macros.sql.ceiling+2]    the span is the whole literal
pub(crate) fn validate(literal: &LitStr) -> Result<pgorm_sql_check::Effect, Error> {
    pgorm_sql_check::classify_sql(&literal.value())
        .map_err(|rejection| Error::new(literal.span(), rejection))
}

/// `text` as the `&'static pgorm::Sql<K>` its effect selects, `K` being
/// `pgorm::Reads` or `pgorm::Writes`. Every token is spanned at the
/// invocation, so a lint judging where an expression came from sees the macro.
///
/// The value is a `const` item's, read by the block's tail rather than given
/// as an inline `const { .. }`: a caller expecting `&str` coerces the tail,
/// which runs, where it could not coerce inside a constant.
// [spec:pgorm:def:read-only.sql-macro]
pub(crate) fn typed(text: &LitStr, effect: pgorm_sql_check::Effect) -> TokenStream {
    let mut literal = text.clone();
    literal.set_span(proc_macro2::Span::call_site());
    let kind = match effect {
        pgorm_sql_check::Effect::Reads => quote!(::pgorm::Reads),
        pgorm_sql_check::Effect::Writes => quote!(::pgorm::Writes),
    };
    let name = proc_macro2::Ident::new("CHECKED", proc_macro2::Span::mixed_site());
    quote!({
        const #name: &::pgorm::Sql<#kind> = &::pgorm::Sql::<#kind>::__checked(#literal);
        #name
    })
}
