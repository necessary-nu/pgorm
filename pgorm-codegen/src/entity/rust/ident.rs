use crate::Error;
use proc_macro2::{Ident, TokenStream, TokenTree};
use quote::format_ident;

/// `format_ident!` panics on anything that is not a legal Rust identifier, so
/// every DB-derived name is put through here while the Rust writer's gate can
/// still return the failure. `context` names where the identifier came from.
// [spec:pgorm:sem:codegen.entity.keywords+2]
pub(crate) fn safe_ident(context: &str, raw: &str) -> Result<Ident, Error> {
    if is_ident(raw) {
        Ok(format_ident!("{}", raw))
    } else {
        Err(Error::TransformError(format!(
            "{context}: `{raw}` is not a valid Rust identifier"
        )))
    }
}

/// True when `raw` lexes as exactly one identifier token and nothing else —
/// the same shape `format_ident!` accepts, raw identifiers (`r#type`) included.
fn is_ident(raw: &str) -> bool {
    let Ok(stream) = raw.parse::<TokenStream>() else {
        return false;
    };
    let mut tokens = stream.into_iter();
    let one_ident = matches!(tokens.next(), Some(TokenTree::Ident(ident)) if ident == raw);
    one_ident && tokens.next().is_none()
}
