use crate::Error;
use proc_macro2::{Delimiter, Ident, Spacing, TokenStream, TokenTree};
use quote::format_ident;

/// `format_ident!` panics on anything that is not a legal Rust identifier, so
/// every DB-derived name is put through here while the transform gate can still
/// return the failure. `context` names where the identifier came from.
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

/// The first column a key names a second time, compared as the text the
/// server reads. PostgreSQL refuses such a key (`42701`), and the entity a
/// transform would read from it is not the key written: one `PrimaryKey`
/// variant twice over, or a unique key whose set of columns is narrower than
/// its list.
// [spec:pgorm:req:codegen.ddl.unsupported+17]
// [spec:pgorm:sem:codegen.entity.transform+16]
pub(crate) fn repeated_column<I>(columns: I) -> Option<String>
where
    I: IntoIterator<Item = String>,
{
    let mut seen = std::collections::BTreeSet::new();
    columns
        .into_iter()
        .find(|column| !seen.insert(column.clone()))
}

/// `tokens` as the Rust a person writes: no space around `::`, `.` or
/// brackets, one after each comma and after a field's `:`. The writers carry
/// an expression or a column type as the text of an attribute, which no
/// formatter reaches, so it is spaced here.
// [spec:pgorm:sem:codegen.entity.expressions+1]
pub(crate) fn rust_source(tokens: &TokenStream) -> String {
    let mut source = String::new();
    write_source(tokens, &mut source);
    source
}

fn write_source(tokens: &TokenStream, source: &mut String) {
    let mut words = false;
    // The first `:` of a `::` is joined to the next; a `:` that closes one
    // is not, so whether one is open decides which a lone `:` is.
    let mut path = false;
    for token in tokens.clone() {
        let word = matches!(token, TokenTree::Ident(_) | TokenTree::Literal(_));
        let after_word = words;
        if word && words {
            source.push(' ');
        }
        words = word;
        match token {
            TokenTree::Group(group) => {
                let (open, close) = match group.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::Brace if after_word => (" { ", " }"),
                    Delimiter::Brace => ("{ ", " }"),
                    Delimiter::None => ("", ""),
                };
                source.push_str(open);
                write_source(&group.stream(), source);
                if group.delimiter() == Delimiter::Brace && source.ends_with(", ") {
                    source.truncate(source.len() - 2);
                }
                source.push_str(close);
            }
            TokenTree::Punct(punct) if punct.as_char() == ',' => source.push_str(", "),
            TokenTree::Punct(punct) if punct.as_char() == ':' => {
                let joint = punct.spacing() == Spacing::Joint;
                source.push_str(if joint || path { ":" } else { ": " });
                path = joint;
            }
            TokenTree::Punct(punct) => source.push(punct.as_char()),
            other => source.push_str(&other.to_string()),
        }
    }
}

// [spec:pgorm:sem:codegen.entity.keywords+2]
pub(crate) fn escape_rust_keyword<T>(string: T) -> String
where
    T: ToString,
{
    let string = string.to_string();
    if RUST_KEYWORDS.iter().any(|s| s.eq(&string)) {
        format!("r#{string}")
    } else if RUST_SPECIAL_KEYWORDS.iter().any(|s| s.eq(&string)) {
        format!("{string}_")
    } else {
        string
    }
}

pub(crate) const RUST_KEYWORDS: [&str; 49] = [
    "as", "async", "await", "break", "const", "continue", "dyn", "else", "enum", "extern", "false",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "static", "struct", "super", "trait", "true", "type", "union", "unsafe", "use",
    "where", "while", "abstract", "become", "box", "do", "final", "macro", "override", "priv",
    "try", "typeof", "unsized", "virtual", "yield",
];

pub(crate) const RUST_SPECIAL_KEYWORDS: [&str; 3] = ["crate", "Self", "self"];
