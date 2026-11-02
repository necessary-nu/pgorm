use std::iter::FromIterator;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::util::{COMPOSITE_KEYS, bare_code};

/// What the entity's other parts are named, each defaulting to its usual name.
#[derive(Default)]
struct PartNames {
    column: Option<syn::Ident>,
    model: Option<syn::Ident>,
    active_model: Option<syn::Ident>,
    primary_key: Option<syn::Ident>,
    relation: Option<syn::Ident>,
}

impl PartNames {
    /// Read every `#[pgorm(..)]` on the struct. A key this derive does not read
    /// is refused at the key rather than dropped: the composite derive's keys
    /// and `entity` with a message saying where they belong, anything else as
    /// unknown.
    // [spec:pgorm:req:macros.derive.names]
    fn parse(attrs: &[syn::Attribute]) -> syn::Result<Self> {
        let mut names = Self::default();
        for attr in attrs.iter().filter(|attr| attr.path().is_ident("pgorm")) {
            attr.parse_nested_meta(|meta| {
                let key = meta
                    .path
                    .get_ident()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                let slot = match key.as_str() {
                    "column" => &mut names.column,
                    "model" => &mut names.model,
                    "active_model" => &mut names.active_model,
                    "primary_key" => &mut names.primary_key,
                    "relation" => &mut names.relation,
                    "entity" => {
                        return Err(meta.error(
                            "`entity` names the entity a model or relation belongs to; \
                             `EntityTrait` is derived on the entity itself",
                        ));
                    }
                    composite if COMPOSITE_KEYS.contains(&composite) => {
                        return Err(meta.error(format!(
                            "`{key}` is the composite derive's: an entity under \
                             `EntityTrait` implements `EntityName` by hand, its names \
                             computed at run time, so `{key}` goes on the `Model` under \
                             `Entity`"
                        )));
                    }
                    _ => {
                        return Err(meta.error(format!(
                            "unknown pgorm attribute `{key}`: `EntityTrait` reads \
                             `active_model`, `column`, `model`, `primary_key` and `relation`"
                        )));
                    }
                };
                *slot = Some(bare_code(&meta, &key)?);
                Ok(())
            })?;
        }
        Ok(names)
    }
}

struct EntityTrait {
    column_ident: syn::Ident,
    ident: syn::Ident,
    model_ident: syn::Ident,
    active_model_ident: syn::Ident,
    primary_key_ident: syn::Ident,
    relation_ident: syn::Ident,
}

impl EntityTrait {
    fn new(input: syn::DeriveInput) -> Result<Self, syn::Error> {
        let pgorm_attr = PartNames::parse(&input.attrs)?;

        let ident = input.ident;
        let column_ident = pgorm_attr.column.unwrap_or_else(|| format_ident!("Column"));
        let model_ident = pgorm_attr.model.unwrap_or_else(|| format_ident!("Model"));
        let active_model_ident = pgorm_attr
            .active_model
            .unwrap_or_else(|| format_ident!("ActiveModel"));
        let primary_key_ident = pgorm_attr
            .primary_key
            .unwrap_or_else(|| format_ident!("PrimaryKey"));
        let relation_ident = pgorm_attr
            .relation
            .unwrap_or_else(|| format_ident!("Relation"));

        Ok(EntityTrait {
            column_ident,
            ident,
            model_ident,
            active_model_ident,
            primary_key_ident,
            relation_ident,
        })
    }

    fn expand(&self) -> TokenStream {
        let expanded_impl_entity_trait = self.impl_entity_trait();
        let expanded_impl_iden = self.impl_iden();
        let expanded_impl_iden_static = self.impl_iden_static();

        TokenStream::from_iter([
            expanded_impl_entity_trait,
            expanded_impl_iden,
            expanded_impl_iden_static,
        ])
    }

    fn impl_entity_trait(&self) -> TokenStream {
        let Self {
            ident,
            model_ident,
            active_model_ident,
            column_ident,
            primary_key_ident,
            relation_ident,
            ..
        } = self;

        quote!(
            #[automatically_derived]
            impl pgorm::entity::EntityTrait for #ident {
                type Model = #model_ident;

                type ActiveModel = #active_model_ident;

                type Column = #column_ident;

                type PrimaryKey = #primary_key_ident;

                type Relation = #relation_ident;
            }
        )
    }

    // [spec:pgorm:sem:macros.derive.entity+6]
    fn impl_iden(&self) -> TokenStream {
        let ident = &self.ident;

        quote!(
            #[automatically_derived]
            impl pgorm::SqlName for #ident {
                fn unquoted(&self, s: &mut dyn std::fmt::Write) {
                    write!(s, "{}", pgorm::StaticName::as_str(self)).unwrap();
                }

                fn table_schema(&self) -> Option<&str> {
                    <Self as pgorm::EntityName>::schema_name(self)
                }
            }
        )
    }

    fn impl_iden_static(&self) -> TokenStream {
        let ident = &self.ident;

        quote!(
            #[automatically_derived]
            impl pgorm::StaticName for #ident {
                fn as_str(&self) -> &str {
                    <Self as pgorm::EntityName>::table_name(self)
                }
            }
        )
    }
}

// [spec:pgorm:sem:macros.derive.entity+6]
pub fn expand_derive_entity(input: syn::DeriveInput) -> syn::Result<TokenStream> {
    Ok(EntityTrait::new(input)?.expand())
}
