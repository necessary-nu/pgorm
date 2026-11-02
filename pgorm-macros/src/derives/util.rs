use heck::ToUpperCamelCase;
use proc_macro2::Span;
use quote::{ToTokens, format_ident};
use syn::{
    Expr, Field, Ident, LitStr, Meta, Type, meta::ParseNestedMeta, parse::Parse,
    punctuated::Punctuated, token::Comma,
};

/// The refusal for a `#[pgorm(...)]` key none of a derive's arms read, spanned at the
/// key and naming it, so a typo is diagnosed rather than dropped.
// [spec:pgorm:syn:macros.derive.entity-model.attrs+6]
pub(crate) fn unknown_pgorm_key(meta: &ParseNestedMeta<'_>) -> syn::Error {
    let name = match meta.path.get_ident() {
        Some(ident) => ident.to_string(),
        None => meta.path.to_token_stream().to_string().replace(' ', ""),
    };
    meta.error(format!("unknown pgorm attribute `{name}`"))
}

/// The Rust code a key's value is, written bare. A string there is refused at
/// the string, since the quotes would make it data the derive re-parses.
// [spec:pgorm:req:macros.derive.one-form]
// [spec:pgorm:syn:macros.derive.entity-model.attrs+6]
pub(crate) fn bare_code<T: Parse>(meta: &ParseNestedMeta<'_>, key: &str) -> syn::Result<T> {
    let value = meta.value()?;
    if value.peek(LitStr) {
        let string: LitStr = value.parse()?;
        return Err(syn::Error::new(
            string.span(),
            format!("`{key}` is Rust code, written bare: remove the quotes"),
        ));
    }
    value.parse()
}

/// Accept a key this derive recognises but does not read, consuming its `= value` when
/// it carries one so the parse stream advances past it; refuse anything else.
// [spec:pgorm:syn:macros.derive.entity-model.attrs+6]
pub(crate) fn skip_known_key(meta: &ParseNestedMeta<'_>, known: &[&str]) -> syn::Result<()> {
    if !known.iter().any(|key| meta.path.is_ident(key)) {
        return Err(unknown_pgorm_key(meta));
    }
    // Bare markers such as `primary_key` carry no value, so a missing `=` is not an error.
    let _: Option<Expr> = meta.value().and_then(|value| value.parse()).ok();
    Ok(())
}

/// The field-level keys `FromQueryResult` and `PartialModel` define between them.
/// The two are routinely derived on one struct — a partial model is decoded through
/// `FromQueryResult` — so each has to recognise the other's vocabulary.
// [spec:pgorm:sem:macros.derive.from-query-result+3]
// [spec:pgorm:sem:macros.derive.partial-model+4]
pub(crate) const PROJECTION_FIELD_KEYS: [&str; 3] = ["from_col", "from_expr", "skip"];

/// Every field-level `#[pgorm(...)]` key the `Entity` derive defines. `Model` and
/// the active-model expansion read only `enum_name` (and `ignore`, through
/// `field_not_ignored`) out of the set, but the composite derive runs both over the same
/// input, so each has to recognise the whole vocabulary or a legal entity model would stop
/// compiling.
// [spec:pgorm:syn:macros.derive.entity-model.attrs+6]
pub(crate) const MODEL_FIELD_KEYS: [&str; 22] = [
    "auto_increment",
    "column_name",
    "column_type",
    "comment",
    "default_expr",
    "default_value",
    "enum_name",
    "generated_stored",
    "generated_virtual",
    "identity",
    "identity_by_default",
    "ignore",
    "indexed",
    "nullable",
    "primary_key",
    "save_array_with",
    "save_as",
    "save_with",
    "select_as",
    "select_with",
    "unique",
    "without_overlaps",
];

pub(crate) fn field_not_ignored(field: &Field) -> bool {
    for attr in field.attrs.iter() {
        if let Some(ident) = attr.path().get_ident() {
            if ident != "pgorm" {
                continue;
            }
        } else {
            continue;
        }

        if let Ok(list) = attr.parse_args_with(Punctuated::<Meta, Comma>::parse_terminated) {
            for meta in list.iter() {
                if let Meta::Path(path) = meta
                    && let Some(name) = path.get_ident()
                    && name == "ignore"
                {
                    return false;
                }
            }
        }
    }
    true
}

pub(crate) fn format_field_ident(field: Field) -> Ident {
    format_ident!("{}", field.ident.unwrap().to_string())
}

/// A field's type as written, rendered without the spaces tokenisation leaves
/// between its parts, so a diagnostic spells `Option<i32>` rather than
/// `Option < i32 >`.
// [spec:pgorm:def:exec.verify]    the `rust_type` an `ExpectedColumn` carries
pub(crate) fn spell_type(ty: &Type) -> String {
    ty.to_token_stream()
        .to_string()
        .replace(" ::", "::")
        .replace(":: ", "::")
        .replace(" <", "<")
        .replace("< ", "<")
        .replace(" >", ">")
        .replace(" ,", ",")
}

pub(crate) fn trim_starting_raw_identifier<T>(string: T) -> String
where
    T: ToString,
{
    string
        .to_string()
        .trim_start_matches(RAW_IDENTIFIER)
        .to_string()
}

/// Turn a name derived from `field` into an identifier, naming the field and the
/// derived spelling when the derivation produced something unusable — case
/// conversion drops non-alphanumerics, so `__` derives `""` and `_1` derives `"1"`.
// [spec:pgorm:sem:macros.derive.entity-model.casing+3]
pub(crate) fn parse_derived_ident(derived: &str, field: &Ident) -> syn::Result<Ident> {
    syn::parse_str::<Ident>(derived).map_err(|_| {
        syn::Error::new(
            field.span(),
            format!(
                "field `{field}` derives the identifier {derived:?}, which is not a valid Rust identifier"
            ),
        )
    })
}

/// The identifier an `enum_name = "..."` attribute names, reported at the literal
/// when it does not spell one.
// [spec:pgorm:sem:macros.derive.entity-model.casing+3]
pub(crate) fn parse_enum_name(litstr: &syn::LitStr) -> syn::Result<Ident> {
    syn::parse_str::<Ident>(&litstr.value()).map_err(|_| {
        syn::Error::new(
            litstr.span(),
            format!(
                "`enum_name` must be a valid Rust identifier, found {:?}",
                litstr.value()
            ),
        )
    })
}

/// The `Column` variant identifier for a model field, from its UpperCamelCase name.
///
/// Validity is judged on the keyword-escaped spelling because the final variant
/// name is escaped downstream (`SELF` -> `Self` -> `Self_`); the identifier
/// returned keeps the unescaped spelling that `rename_all` converts.
// [spec:pgorm:sem:macros.derive.entity-model.casing+3]
pub(crate) fn column_variant_ident(camel_name: &str, field: &Ident) -> syn::Result<Ident> {
    parse_derived_ident(&escape_rust_keyword(camel_name), field)?;
    Ok(Ident::new(camel_name, Span::call_site()))
}

// [spec:pgorm:sem:macros.derive.entity-model.casing+3]
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

/// Turn a string to PascalCase while escaping all special characters in ASCII words.
///
/// (camel_case is used here to match naming of heck.)
///
/// In ActiveEnum, string_value will be PascalCased and made
/// an identifier in {Enum}Variant.
///
/// However Rust only allows for XID_Start char followed by
/// XID_Continue characters as identifiers; this causes a few
/// problems:
///
/// - `string_value = ""` will cause a panic;
/// - `string_value` containing only non-alphanumerics will become `""`
///   and cause the above panic;
/// - `string_values`:
///      - `"A B"`
///      - `"A  B"`
///      - `"A_B"`
///      - `"A_ B"`
///
/// All shares the same identifier of `"AB"`;
///
/// This function does the PascelCase conversion with a few special escapes:
/// - Non-Unicode Standard Annex #31 compliant characters will converted to their hex notation;
/// - `"_"` into `"0x5F"`;
/// - `" "` into `"0x20"`;
/// - Empty strings will become special keyword of `"__Empty"`
///
/// Note that this does NOT address:
///
/// - case-sensitivity. String value "ABC" and "abc" remains
///   conflicted after .camel_case().
///
/// Example Conversions:
///
/// ```ignore
/// assert_eq!(camel_case_with_escaped_non_uax31(""), "__Empty");
/// assert_eq!(camel_case_with_escaped_non_uax31(" "), "_0x20");
/// assert_eq!(camel_case_with_escaped_non_uax31("  "), "_0x200x20");
/// assert_eq!(camel_case_with_escaped_non_uax31("_"), "_0x5F");
/// assert_eq!(camel_case_with_escaped_non_uax31("foobar"), "Foobar");
/// assert_eq!(camel_case_with_escaped_non_uax31("foo bar"), "Foo0x20bar");
/// ```
pub(crate) fn camel_case_with_escaped_non_uax31<T>(string: T) -> String
where
    T: ToString,
{
    let additional_chars_to_replace: [char; 2] = ['_', ' '];

    let mut rebuilt = string
        .to_string()
        .chars()
        .enumerate()
        .map(|(pos, char_)| {
            if !additional_chars_to_replace.contains(&char_)
                && match pos {
                    0 => unicode_ident::is_xid_start(char_),
                    _ => unicode_ident::is_xid_continue(char_),
                }
            {
                char_.to_string()
            } else {
                format!("{:#X}", char_ as u32)
            }
        })
        .reduce(
            // Join the "characters" (now strings)
            // back together
            |lhs, rhs| lhs + rhs.as_str(),
        )
        .map_or(
            // if string_value is ""
            // Make sure the default does NOT go through camel_case,
            // as the __ will be removed! The underscores are
            // what guarantees this being special case avoiding
            // all potential conflicts.
            String::from("__Empty"),
            |s| s.to_upper_camel_case(),
        );

    if rebuilt
        .chars()
        .next()
        .map(char::is_numeric)
        .unwrap_or(false)
    {
        rebuilt = String::from("_") + &rebuilt;
    }

    rebuilt
}

pub(crate) const RAW_IDENTIFIER: &str = "r#";

pub(crate) const RUST_KEYWORDS: [&str; 49] = [
    "as", "async", "await", "break", "const", "continue", "dyn", "else", "enum", "extern", "false",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "static", "struct", "super", "trait", "true", "type", "union", "unsafe", "use",
    "where", "while", "abstract", "become", "box", "do", "final", "macro", "override", "priv",
    "try", "typeof", "unsized", "virtual", "yield",
];

pub(crate) const RUST_SPECIAL_KEYWORDS: [&str; 3] = ["crate", "Self", "self"];

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:pgorm:sem:macros.derive.active-enum.expansion+3/test]
    #[test]
    fn test_non_uax31_escape() {
        // Test empty string
        assert_eq!(camel_case_with_escaped_non_uax31(""), "__Empty");

        // Test additional_chars_to_replace (to_camel_case related characters)
        assert_eq!(camel_case_with_escaped_non_uax31(" "), "_0x20");

        // Test additional_chars_to_replace (multiples. ensure distinct from single)
        assert_eq!(camel_case_with_escaped_non_uax31("  "), "_0x200x20");

        // Test additional_chars_to_replace (udnerscores)
        assert_eq!(camel_case_with_escaped_non_uax31("_"), "_0x5F");

        // Test typical use case
        assert_eq!(camel_case_with_escaped_non_uax31("foobar"), "Foobar");

        // Test spaced words distinct from non-spaced
        assert_eq!(camel_case_with_escaped_non_uax31("foo bar"), "Foo0x20bar");

        // Test undescored words distinct from non-spaced and spaced
        assert_eq!(camel_case_with_escaped_non_uax31("foo_bar"), "Foo0x5Fbar");

        // Test leading numeric characters
        assert_eq!(camel_case_with_escaped_non_uax31("1"), "_0x31");

        // Test escaping also works on full string following lead numeric character
        // This was previously a fail condition.
        assert_eq!(
            camel_case_with_escaped_non_uax31("1 2 3"),
            "_0x310x2020x203"
        );
    }
}
