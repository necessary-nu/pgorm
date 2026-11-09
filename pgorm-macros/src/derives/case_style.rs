//! A derive's `rename_all` and `rename` keys, read as the case styles
//! `pgorm-case-style` converts by, which pgorm's run-time models rename by too.
use pgorm_case_style::CASE_STYLES;
pub use pgorm_case_style::CaseStyle;
use syn::{Ident, LitStr, meta::ParseNestedMeta};

pub trait CaseStyleHelpers {
    fn convert_case(&self, case_style: Option<CaseStyle>) -> String;
}

impl CaseStyleHelpers for Ident {
    fn convert_case(&self, case_style: Option<CaseStyle>) -> String {
        convert_case_str(&self.to_string(), case_style)
    }
}

/// The same conversion for a name already in hand as a string, which a field's
/// identifier is once its raw-identifier prefix has been trimmed off.
pub fn convert_case_str(text: &str, case_style: Option<CaseStyle>) -> String {
    match case_style {
        Some(case_style) => case_style.convert(text),
        None => text.to_owned(),
    }
}

/// The case style a `rename_all = ".."` or `rename = ".."` key names.
pub fn case_style_of(meta: &ParseNestedMeta<'_>) -> syn::Result<CaseStyle> {
    let meta_string_literal: LitStr = meta.value()?.parse()?;
    let value_string = meta_string_literal.value();
    value_string.parse().map_err(|_| {
        meta.error(format!(
            "Unknown value for attribute parameter: `{value_string}`. Valid values are: `{CASE_STYLES:?}`"
        ))
    })
}

#[test]
fn test_convert_case() {
    let id = Ident::new("test_me", proc_macro2::Span::call_site());
    assert_eq!("testMe", id.convert_case(Some(CaseStyle::CamelCase)));
    assert_eq!("TestMe", id.convert_case(Some(CaseStyle::PascalCase)));
}
