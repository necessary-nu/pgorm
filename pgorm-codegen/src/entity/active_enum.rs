use heck::{ToSnakeCase, ToUpperCamelCase};
use pgorm_query::Name;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::fmt::Write;

use crate::{Error, WithSerde, util::safe_ident};

#[derive(Clone, Debug)]
pub struct ActiveEnum {
    pub(crate) enum_name: Name,
    pub(crate) schema: Option<Name>,
    pub(crate) values: Vec<Name>,
}

/// The Rust variant name a DB enum value maps to, and whether deriving it
/// needed the per-character encoding fallback.
struct Variant {
    name: String,
    encoded: bool,
}

// [spec:pgorm:sem:codegen.entity.enums+6]
fn variant(value: &str) -> Variant {
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

impl ActiveEnum {
    /// Each DB value paired with the Rust variant name it derives, read the way
    /// the generated variants are: trimmed, in declaration order.
    // [spec:pgorm:req:codegen.entity.collisions+2]
    pub(crate) fn variant_names(&self) -> Vec<(String, String)> {
        self.values
            .iter()
            .map(|value| {
                let value = value.to_string().trim().to_owned();
                let name = variant(&value).name;
                (value, name)
            })
            .collect()
    }

    // [spec:pgorm:sem:codegen.entity.enums+6]
    // [spec:pgorm:sem:codegen.entity.keywords+2]
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let enum_name = self.enum_name.to_string();
        let context = format!("enum `{enum_name}`");
        safe_ident(&context, &enum_name.to_upper_camel_case())?;
        for value in self.values.iter().map(|value| value.to_string()) {
            let value = value.trim();
            let variant = variant(value);
            if variant.encoded {
                println!(
                    "Warning: item '{value}' in the enumeration '{enum_name}' cannot be converted into a valid Rust enum member name. It will be converted to its corresponding UTF-8 encoding. You can modify it later as needed."
                );
            }
            safe_ident(&format!("{context} value `{value}`"), &variant.name)?;
        }
        Ok(())
    }

    /// The enum as the `ActiveEnum` derive reads it, saying only what differs
    /// from the derive's defaults: `enum_name` where the type is not named
    /// the enum's name in snake_case, and `string_value` where a value is not
    /// its variant's name in snake_case.
    // [spec:pgorm:sem:codegen.entity.enums+6]
    // [spec:pgorm:sem:macros.derive.boilerplate]
    pub fn impl_active_enum(
        &self,
        with_serde: &WithSerde,
        with_copy_enums: bool,
        extra_derives: &TokenStream,
        extra_attributes: &TokenStream,
    ) -> TokenStream {
        let enum_name = &self.enum_name.to_string();
        let enum_iden = format_ident!("{}", enum_name.to_upper_camel_case());
        let variants = self.values.iter().map(|value| {
            let value = value.to_string();
            let variant = variant(value.trim()).name;
            let iden = format_ident!("{}", variant);
            if variant.to_snake_case() == value {
                quote! { #iden }
            } else {
                quote! {
                    #[pgorm(string_value = #value)]
                    #iden
                }
            }
        });

        let serde_derive = with_serde.extra_derive();
        let copy_derive = if with_copy_enums {
            quote! { , Copy }
        } else {
            quote! {}
        };

        let mut keys = Vec::new();
        if enum_iden.to_string().to_snake_case() != *enum_name {
            keys.push(quote! { enum_name = #enum_name });
        }
        if let Some(schema) = &self.schema {
            let schema = schema.to_string();
            keys.push(quote! { schema_name = #schema });
        }
        let keys = (!keys.is_empty()).then(|| quote! { #[pgorm(#(#keys),*)] });

        quote! {
            #[derive(Debug, Clone, PartialEq, Eq, ActiveEnum #copy_derive #serde_derive #extra_derives)]
            #keys
            #extra_attributes
            pub enum #enum_iden {
                #( #variants, )*
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::writer::{bonus_attributes, bonus_derive};

    use pretty_assertions::assert_eq;

    #[test]
    fn test_enum_variant_starts_with_number() {
        assert_eq!(
            ActiveEnum {
                schema: None,
                enum_name: Name::runtime("media_type"),
                values: vec![
                    "UNKNOWN",
                    "BITMAP",
                    "DRAWING",
                    "AUDIO",
                    "VIDEO",
                    "MULTIMEDIA",
                    "OFFICE",
                    "TEXT",
                    "EXECUTABLE",
                    "ARCHIVE",
                    "3D",
                ]
                .into_iter()
                .map(Name::runtime)
                .collect(),
            }
            .impl_active_enum(
                &WithSerde::None,
                true,
                &TokenStream::new(),
                &TokenStream::new(),
            )
            .to_string(),
            quote!(
                #[derive(Debug, Clone, PartialEq, Eq, ActiveEnum, Copy)]
                pub enum MediaType {
                    #[pgorm(string_value = "UNKNOWN")]
                    Unknown,
                    #[pgorm(string_value = "BITMAP")]
                    Bitmap,
                    #[pgorm(string_value = "DRAWING")]
                    Drawing,
                    #[pgorm(string_value = "AUDIO")]
                    Audio,
                    #[pgorm(string_value = "VIDEO")]
                    Video,
                    #[pgorm(string_value = "MULTIMEDIA")]
                    Multimedia,
                    #[pgorm(string_value = "OFFICE")]
                    Office,
                    #[pgorm(string_value = "TEXT")]
                    Text,
                    #[pgorm(string_value = "EXECUTABLE")]
                    Executable,
                    #[pgorm(string_value = "ARCHIVE")]
                    Archive,
                    #[pgorm(string_value = "3D")]
                    _3D,
                }
            )
            .to_string()
        )
    }

    #[test]
    fn test_enum_extra_derives() {
        assert_eq!(
            ActiveEnum {
                schema: None,
                enum_name: Name::runtime("media_type"),
                values: vec!["UNKNOWN", "BITMAP",]
                    .into_iter()
                    .map(Name::runtime)
                    .collect(),
            }
            .impl_active_enum(
                &WithSerde::None,
                true,
                &bonus_derive("enum_extra_derives", ["specta::Type", "ts_rs::TS"])
                    .expect("valid derives"),
                &TokenStream::new(),
            )
            .to_string(),
            build_generated_enum(),
        );

        #[rustfmt::skip]
        fn build_generated_enum() -> String {
            quote!(
                #[derive(Debug, Clone, PartialEq, Eq, ActiveEnum, Copy, specta :: Type, ts_rs :: TS)]
                pub enum MediaType {
                    #[pgorm(string_value = "UNKNOWN")]
                    Unknown,
                    #[pgorm(string_value = "BITMAP")]
                    Bitmap,
                }
            )
            .to_string()
        }
    }

    #[test]
    fn test_enum_extra_attributes() {
        assert_eq!(
            ActiveEnum {
                schema: None,
                enum_name: Name::runtime("coinflip_result_type"),
                values: vec!["HEADS", "TAILS"]
                    .into_iter()
                    .map(Name::runtime)
                    .collect(),
            }
            .impl_active_enum(
                &WithSerde::None,
                true,
                &TokenStream::new(),
                &bonus_attributes(
                    "enum_extra_attributes",
                    [r#"serde(rename_all = "camelCase")"#]
                )
                .expect("valid attributes")
            )
            .to_string(),
            quote!(
                #[derive(Debug, Clone, PartialEq, Eq, ActiveEnum, Copy)]
                #[serde(rename_all = "camelCase")]
                pub enum CoinflipResultType {
                    #[pgorm(string_value = "HEADS")]
                    Heads,
                    #[pgorm(string_value = "TAILS")]
                    Tails,
                }
            )
            .to_string()
        );
        assert_eq!(
            ActiveEnum {
                schema: None,
                enum_name: Name::runtime("coinflip_result_type"),
                values: vec!["HEADS", "TAILS"]
                    .into_iter()
                    .map(Name::runtime)
                    .collect(),
            }
            .impl_active_enum(
                &WithSerde::None,
                true,
                &TokenStream::new(),
                &bonus_attributes(
                    "enum_extra_attributes",
                    [r#"serde(rename_all = "camelCase")"#, "ts(export)"]
                )
                .expect("valid attributes")
            )
            .to_string(),
            quote!(
                #[derive(Debug, Clone, PartialEq, Eq, ActiveEnum, Copy)]
                #[serde(rename_all = "camelCase")]
                #[ts(export)]
                pub enum CoinflipResultType {
                    #[pgorm(string_value = "HEADS")]
                    Heads,
                    #[pgorm(string_value = "TAILS")]
                    Tails,
                }
            )
            .to_string()
        )
    }

    #[test]
    fn test_enum_variant_utf8_encode() {
        assert_eq!(
            ActiveEnum {
                schema: None,
                enum_name: Name::runtime("ty"),
                values: vec![
                    "Question",
                    "QuestionsAdditional",
                    "Answer",
                    "Other",
                    "/",
                    "//",
                    "A-B-C",
                    "你好",
                ]
                .into_iter()
                .map(Name::runtime)
                .collect(),
            }
            .impl_active_enum(
                &WithSerde::None,
                true,
                &TokenStream::new(),
                &TokenStream::new(),
            )
            .to_string(),
            quote!(
                #[derive(Debug, Clone, PartialEq, Eq, ActiveEnum, Copy)]
                pub enum Ty {
                    #[pgorm(string_value = "Question")]
                    Question,
                    #[pgorm(string_value = "QuestionsAdditional")]
                    QuestionsAdditional,
                    #[pgorm(string_value = "Answer")]
                    Answer,
                    #[pgorm(string_value = "Other")]
                    Other,
                    #[pgorm(string_value = "/")]
                    U002F,
                    #[pgorm(string_value = "//")]
                    U002FU002F,
                    #[pgorm(string_value = "A-B-C")]
                    ABC,
                    你好,
                }
            )
            .to_string()
        )
    }

    // [spec:pgorm:sem:codegen.entity.enums+6/test]    `enum_name` and `string_value` only where they differ from the derive's snake_case defaults
    #[test]
    fn test_enum_defaults_are_left_unsaid() {
        assert_eq!(
            ActiveEnum {
                schema: Some(Name::runtime("crm")),
                enum_name: Name::runtime("Mood"),
                values: vec!["calm", "very_busy", "Loud"]
                    .into_iter()
                    .map(Name::runtime)
                    .collect(),
            }
            .impl_active_enum(
                &WithSerde::None,
                false,
                &TokenStream::new(),
                &TokenStream::new(),
            )
            .to_string(),
            quote!(
                #[derive(Debug, Clone, PartialEq, Eq, ActiveEnum)]
                #[pgorm(enum_name = "Mood", schema_name = "crm")]
                pub enum Mood {
                    Calm,
                    VeryBusy,
                    #[pgorm(string_value = "Loud")]
                    Loud,
                }
            )
            .to_string()
        );
        assert_eq!(
            ActiveEnum {
                schema: None,
                enum_name: Name::runtime("tea"),
                values: vec!["everyday"].into_iter().map(Name::runtime).collect(),
            }
            .impl_active_enum(
                &WithSerde::None,
                false,
                &TokenStream::new(),
                &TokenStream::new(),
            )
            .to_string(),
            quote!(
                #[derive(Debug, Clone, PartialEq, Eq, ActiveEnum)]
                pub enum Tea {
                    Everyday,
                }
            )
            .to_string()
        );
    }
}
