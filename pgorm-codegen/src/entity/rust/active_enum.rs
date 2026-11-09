use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use tracing::warn;

use super::ident::safe_ident;
use crate::{ActiveEnum, Error, WithSerde, entity::active_enum::variant};

impl ActiveEnum {
    /// Each DB value paired with the Rust variant name it derives, read the way
    /// the generated variants are: trimmed, in declaration order.
    // [spec:pgorm:req:codegen.entity.collisions+3]
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

    // [spec:pgorm:sem:codegen.entity.enums+10]
    // [spec:pgorm:sem:codegen.entity.keywords+2]
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let enum_name = self.enum_name.to_string();
        let context = format!("enum `{enum_name}`");
        safe_ident(&context, &enum_name.to_upper_camel_case())?;
        for value in self.values.iter().map(|value| value.to_string()) {
            let value = value.trim();
            let variant = variant(value);
            if variant.encoded {
                warn!(
                    enum_name,
                    value,
                    variant = variant.name,
                    "enum value has no Rust name of its own, so its variant is named for its characters"
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
    ///
    /// Its variants are named by the schema, for the type's values, so
    /// clippy's `enum_variant_names`, which values sharing a word (`web_shop`,
    /// `pop_up_shop`, `closed_shop`) trip, is allowed on it.
    // [spec:pgorm:sem:codegen.entity.enums+10]
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
        let copy_derive = with_copy_enums.then(|| quote! { , Copy });

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
            #[allow(clippy::enum_variant_names)]
            #[derive(Clone #copy_derive, Debug, PartialEq, Eq, ActiveEnum #serde_derive #extra_derives)]
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
    use crate::entity::rust::writer::{bonus_attributes, bonus_derive};
    use pgorm_query::Name;

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
                #[allow(clippy::enum_variant_names)]
                #[derive(Clone, Copy, Debug, PartialEq, Eq, ActiveEnum)]
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
                #[allow(clippy::enum_variant_names)]
                #[derive(Clone, Copy, Debug, PartialEq, Eq, ActiveEnum, specta :: Type, ts_rs :: TS)]
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
                #[allow(clippy::enum_variant_names)]
                #[derive(Clone, Copy, Debug, PartialEq, Eq, ActiveEnum)]
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
                #[allow(clippy::enum_variant_names)]
                #[derive(Clone, Copy, Debug, PartialEq, Eq, ActiveEnum)]
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

    /// An event a test's code reported: its level and its fields as
    /// `name=value`.
    type Event = (tracing::Level, Vec<String>);

    /// The events a test's code reports through `tracing`.
    #[derive(Clone, Default)]
    struct Recorded(std::sync::Arc<std::sync::Mutex<Vec<Event>>>);

    struct Fields(Vec<String>);

    impl tracing::field::Visit for Fields {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.push(format!("{}={value:?}", field.name()));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.push(format!("{}={value}", field.name()));
        }
    }

    impl tracing::Subscriber for Recorded {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }

        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

        fn event(&self, event: &tracing::Event<'_>) {
            let mut fields = Fields(Vec::new());
            event.record(&mut fields);
            if let Ok(mut events) = self.0.lock() {
                events.push((*event.metadata().level(), fields.0));
            }
        }

        fn enter(&self, _: &tracing::span::Id) {}

        fn exit(&self, _: &tracing::span::Id) {}
    }

    // [spec:pgorm:sem:codegen.entity.enums+10/test]    a value encoded per character is reported as a `tracing` warning naming the enum, the value and the variant, the crate printing nothing
    #[test]
    fn an_encoded_value_is_a_tracing_warning() {
        let recorded = Recorded::default();
        let active_enum = ActiveEnum {
            schema: None,
            enum_name: Name::runtime("ty"),
            values: vec![Name::runtime("Answer"), Name::runtime("/")],
        };
        tracing::subscriber::with_default(recorded.clone(), || active_enum.validate())
            .expect("the enum is valid");
        let events = recorded
            .0
            .lock()
            .expect("no test panicked holding it")
            .clone();
        assert_eq!(events.len(), 1, "{events:?}");
        let (level, fields) = &events[0];
        assert_eq!(*level, tracing::Level::WARN);
        for field in ["enum_name=ty", "value=/", "variant=U002F"] {
            assert!(
                fields.iter().any(|recorded| recorded == field),
                "{fields:?}"
            );
        }
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
                #[allow(clippy::enum_variant_names)]
                #[derive(Clone, Copy, Debug, PartialEq, Eq, ActiveEnum)]
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

    // [spec:pgorm:sem:codegen.entity.enums+10/test]    `enum_name` and `string_value` only where they differ from the derive's snake_case defaults
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
                #[allow(clippy::enum_variant_names)]
                #[derive(Clone, Debug, PartialEq, Eq, ActiveEnum)]
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
                #[allow(clippy::enum_variant_names)]
                #[derive(Clone, Debug, PartialEq, Eq, ActiveEnum)]
                pub enum Tea {
                    Everyday,
                }
            )
            .to_string()
        );
    }
}
