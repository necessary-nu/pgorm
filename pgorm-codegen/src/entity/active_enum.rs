use heck::ToUpperCamelCase;
use pgorm_query::Name;
use std::fmt::Write;

#[derive(Clone, Debug)]
pub struct ActiveEnum {
    pub(crate) enum_name: Name,
    pub(crate) schema: Option<Name>,
    pub(crate) values: Vec<Name>,
}

/// The Rust variant name a DB enum value maps to, and whether deriving it
/// needed the per-character encoding fallback.
pub(crate) struct Variant {
    pub(crate) name: String,
    #[cfg_attr(not(feature = "rust"), allow(dead_code))]
    pub(crate) encoded: bool,
}

// [spec:pgorm:sem:codegen.entity.enums+10]
pub(crate) fn variant(value: &str) -> Variant {
    if value.chars().next().map(char::is_numeric).unwrap_or(false) {
        return Variant {
            name: format!("_{value}"),
            encoded: false,
        };
    }
    let camel_case = value.to_upper_camel_case();
    if !camel_case.is_empty() {
        return Variant {
            name: camel_case,
            encoded: false,
        };
    }
    let mut name = String::new();
    for c in value.chars() {
        if c.len_utf8() > 1 {
            let _ = write!(&mut name, "{c}");
        } else {
            let _ = write!(&mut name, "U{:04X}", c as u32);
        }
    }
    Variant {
        name,
        encoded: true,
    }
}

/// The Rust variant name a DB enum value maps to.
// [spec:pgorm:sem:codegen.entity.enums+10]
pub(crate) fn variant_name(value: &str) -> String {
    variant(value).name
}
