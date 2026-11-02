use std::iter::FromIterator;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::attributes::derive_attr;

struct DeriveEntity {
    column_ident: syn::Ident,
    ident: syn::Ident,
    model_ident: syn::Ident,
    active_model_ident: syn::Ident,
    primary_key_ident: syn::Ident,
    relation_ident: syn::Ident,
}

impl DeriveEntity {
    fn new(input: syn::DeriveInput) -> Result<Self, syn::Error> {
        let pgorm_attr = derive_attr::Pgorm::try_from_attributes(&input.attrs)?.unwrap_or_default();
        // [spec:pgorm:req:macros.derive.one-form]
        if let Some(name) = [
            &pgorm_attr.table_name,
            &pgorm_attr.schema_name,
            &pgorm_attr.comment,
        ]
        .into_iter()
        .flatten()
        .next()
        {
            return Err(syn::Error::new_spanned(
                name,
                "an entity under `DeriveEntity` implements `EntityName` by hand, its names \
                 computed at run time; a name written here is the composite derive's: \
                 put it on the `Model` under `DeriveEntityModel`",
            ));
        }

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

        Ok(DeriveEntity {
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

    // [spec:pgorm:sem:macros.derive.entity+4]
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

// [spec:pgorm:sem:macros.derive.entity+4]
pub fn expand_derive_entity(input: syn::DeriveInput) -> syn::Result<TokenStream> {
    Ok(DeriveEntity::new(input)?.expand())
}
