//! The case styles a `rename_all` renames by.
//!
//! `pgorm-macros` renames an entity's columns and an enum's values by them as
//! it expands a derive, and `pgorm` re-exports [`CaseStyle`] so a model a
//! language binding declares at run time is renamed by the same conversion.

use std::{fmt, str::FromStr};

use heck::{
    ToKebabCase, ToLowerCamelCase, ToShoutySnakeCase, ToSnakeCase, ToTitleCase, ToUpperCamelCase,
};

/// A case style a name is rewritten in.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaseStyle {
    /// `camelCase`: `bake_minutes` is `bakeMinutes`.
    CamelCase,
    /// `kebab-case`: `bake_minutes` is `bake-minutes`.
    KebabCase,
    /// `mixed_case`, which is `camelCase` again.
    MixedCase,
    /// `SCREAMING_SNAKE_CASE`: `bakeMinutes` is `BAKE_MINUTES`.
    ShoutySnakeCase,
    /// `snake_case`: `bakeMinutes` is `bake_minutes`.
    SnakeCase,
    /// `title_case`, words capitalised and separated by spaces.
    TitleCase,
    /// `UPPERCASE`, the name's letters made upper case and nothing else.
    UpperCase,
    /// `lowercase`, the name's letters made lower case and nothing else.
    LowerCase,
    /// `SCREAMING-KEBAB-CASE`: `bake_minutes` is `BAKE-MINUTES`.
    ScreamingKebabCase,
    /// `PascalCase`: `bake_minutes` is `BakeMinutes`.
    PascalCase,
}

/// The spelling of each style a `rename_all` is documented to take; the
/// parse also takes the aliases strum's did (`camel_case` for `PascalCase`,
/// `snek_case`, `kebab_case`, `shouty_snake_case`).
pub const CASE_STYLES: &[&str] = &[
    "camelCase",
    "PascalCase",
    "kebab-case",
    "snake_case",
    "SCREAMING_SNAKE_CASE",
    "SCREAMING-KEBAB-CASE",
    "lowercase",
    "UPPERCASE",
    "title_case",
    "mixed_case",
];

/// A name that is no case style's, which [`CaseStyle::from_str`] refuses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownCaseStyle(pub String);

impl fmt::Display for UnknownCaseStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is no case style: rename_all takes one of {CASE_STYLES:?}",
            self.0
        )
    }
}

impl std::error::Error for UnknownCaseStyle {}

impl FromStr for CaseStyle {
    type Err = UnknownCaseStyle;

    fn from_str(text: &str) -> Result<Self, UnknownCaseStyle> {
        Ok(match text {
            "camel_case" | "PascalCase" => CaseStyle::PascalCase,
            "camelCase" => CaseStyle::CamelCase,
            "snake_case" | "snek_case" => CaseStyle::SnakeCase,
            "kebab_case" | "kebab-case" => CaseStyle::KebabCase,
            "SCREAMING-KEBAB-CASE" => CaseStyle::ScreamingKebabCase,
            "shouty_snake_case" | "shouty_snek_case" | "SCREAMING_SNAKE_CASE" => {
                CaseStyle::ShoutySnakeCase
            }
            "title_case" => CaseStyle::TitleCase,
            "mixed_case" => CaseStyle::MixedCase,
            "lowercase" => CaseStyle::LowerCase,
            "UPPERCASE" => CaseStyle::UpperCase,
            _ => return Err(UnknownCaseStyle(text.to_owned())),
        })
    }
}

impl CaseStyle {
    /// `text` written in this style.
    ///
    /// ```
    /// use pgorm_case_style::CaseStyle;
    ///
    /// assert_eq!(CaseStyle::CamelCase.convert("bake_minutes"), "bakeMinutes");
    /// assert_eq!(CaseStyle::PascalCase.convert("bake_minutes"), "BakeMinutes");
    /// assert_eq!(CaseStyle::ScreamingKebabCase.convert("bake_minutes"), "BAKE-MINUTES");
    /// ```
    pub fn convert(&self, text: &str) -> String {
        match self {
            CaseStyle::PascalCase => text.to_upper_camel_case(),
            CaseStyle::KebabCase => text.to_kebab_case(),
            CaseStyle::MixedCase => text.to_lower_camel_case(),
            CaseStyle::ShoutySnakeCase => text.to_shouty_snake_case(),
            CaseStyle::SnakeCase => text.to_snake_case(),
            CaseStyle::TitleCase => text.to_title_case(),
            CaseStyle::UpperCase => text.to_uppercase(),
            CaseStyle::LowerCase => text.to_lowercase(),
            CaseStyle::ScreamingKebabCase => text.to_kebab_case().to_uppercase(),
            CaseStyle::CamelCase => {
                let camel_case = text.to_upper_camel_case();
                let mut pascal = String::with_capacity(camel_case.len());
                let mut it = camel_case.chars();
                if let Some(ch) = it.next() {
                    pascal.extend(ch.to_lowercase());
                }
                pascal.extend(it);
                pascal
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_documented_spelling_parses() {
        for name in CASE_STYLES {
            assert!(name.parse::<CaseStyle>().is_ok(), "{name}");
        }
        assert_eq!(
            "Camel".parse::<CaseStyle>(),
            Err(UnknownCaseStyle("Camel".to_owned()))
        );
    }

    #[test]
    fn each_style_converts() {
        let cases = [
            (CaseStyle::CamelCase, "testMe"),
            (CaseStyle::PascalCase, "TestMe"),
            (CaseStyle::KebabCase, "test-me"),
            (CaseStyle::MixedCase, "testMe"),
            (CaseStyle::ShoutySnakeCase, "TEST_ME"),
            (CaseStyle::SnakeCase, "test_me"),
            (CaseStyle::TitleCase, "Test Me"),
            (CaseStyle::UpperCase, "TEST_ME"),
            (CaseStyle::LowerCase, "test_me"),
            (CaseStyle::ScreamingKebabCase, "TEST-ME"),
        ];
        for (style, expected) in cases {
            assert_eq!(style.convert("test_me"), expected, "{style:?}");
        }
    }
}
