use super::{
    attributes::derive_attr,
    util::{
        MODEL_FIELD_KEYS, escape_rust_keyword, field_not_ignored, parse_derived_ident,
        parse_enum_name, skip_known_key, spell_type, trim_starting_raw_identifier,
    },
};
use heck::ToUpperCamelCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use std::iter::FromIterator;
use syn::{Ident, LitStr};

enum Error {
    InputNotStruct,
    Syn(syn::Error),
}

struct Model {
    column_idents: Vec<syn::Ident>,
    entity_ident: syn::Ident,
    field_idents: Vec<syn::Ident>,
    field_types: Vec<syn::Type>,
    ident: syn::Ident,
    ignore_attrs: Vec<bool>,
}

impl Model {
    fn new(input: syn::DeriveInput) -> Result<Self, Error> {
        let fields = match input.data {
            syn::Data::Struct(syn::DataStruct {
                fields: syn::Fields::Named(syn::FieldsNamed { named, .. }),
                ..
            }) => named,
            _ => return Err(Error::InputNotStruct),
        };

        let pgorm_attr = derive_attr::Pgorm::try_from_attributes(&input.attrs)
            .map_err(Error::Syn)?
            .unwrap_or_default();

        let ident = input.ident;
        let entity_ident = pgorm_attr.entity.unwrap_or_else(|| format_ident!("Entity"));

        let field_idents = fields
            .iter()
            .map(|field| field.ident.as_ref().unwrap().clone())
            .collect();

        let field_types = fields.iter().map(|field| field.ty.clone()).collect();

        let column_idents = fields
            .iter()
            .map(|field| {
                let field_ident = field.ident.as_ref().unwrap();
                let name = trim_starting_raw_identifier(field_ident).to_upper_camel_case();
                let mut ident = parse_derived_ident(&escape_rust_keyword(name), field_ident)
                    .map_err(Error::Syn)?;
                field
                    .attrs
                    .iter()
                    .filter(|attr| attr.path().is_ident("pgorm"))
                    .try_for_each(|attr| {
                        attr.parse_nested_meta(|meta| {
                            if meta.path.is_ident("enum_name") {
                                ident = parse_enum_name(&meta.value()?.parse::<LitStr>()?)?;
                            } else {
                                skip_known_key(&meta, &MODEL_FIELD_KEYS)?;
                            }

                            Ok(())
                        })
                        .map_err(Error::Syn)
                    })?;
                Ok(ident)
            })
            .collect::<Result<_, _>>()?;

        let ignore_attrs = fields
            .iter()
            .map(|field| !field_not_ignored(field))
            .collect();

        Ok(Model {
            column_idents,
            entity_ident,
            field_idents,
            field_types,
            ident,
            ignore_attrs,
        })
    }

    fn expand(&self) -> syn::Result<TokenStream> {
        let expanded_impl_from_query_result = self.impl_from_query_result();
        let expanded_impl_model_trait = self.impl_model_trait();

        Ok(TokenStream::from_iter([
            expanded_impl_from_query_result,
            expanded_impl_model_trait,
        ]))
    }

    fn impl_from_query_result(&self) -> TokenStream {
        let ident = &self.ident;
        let field_idents = &self.field_idents;
        let column_idents = &self.column_idents;
        let field_values: Vec<TokenStream> = column_idents
            .iter()
            .zip(&self.ignore_attrs)
            .map(|(column_ident, ignore)| {
                if *ignore {
                    quote! {
                        Default::default()
                    }
                } else {
                    quote! {
                        row.try_get(pre, pgorm::StaticName::as_str(&<<Self as pgorm::ModelTrait>::Entity as pgorm::entity::EntityTrait>::Column::#column_ident).into())?
                    }
                }
            })
            .collect();

        // [spec:pgorm:sem:macros.derive.model+4]    the reflected column shape
        let expected_columns: Vec<TokenStream> = column_idents
            .iter()
            .zip(&self.field_types)
            .zip(&self.ignore_attrs)
            .filter(|(_, ignore)| !**ignore)
            .map(|((column_ident, field_type), _)| {
                let rust_type = spell_type(field_type);
                quote! {
                    pgorm::ExpectedColumn::new(
                        pgorm::StaticName::as_str(&<<Self as pgorm::ModelTrait>::Entity as pgorm::entity::EntityTrait>::Column::#column_ident),
                        #rust_type,
                        <#field_type as pgorm::TryGetable>::accepts,
                    )
                }
            })
            .collect();

        quote!(
            #[automatically_derived]
            impl pgorm::FromQueryResult for #ident {
                fn from_query_result(row: &pgorm::QueryResult, pre: &str) -> std::result::Result<Self, pgorm::Error> {
                    Ok(Self {
                        #(#field_idents: #field_values),*
                    })
                }

                fn expected_columns() -> std::option::Option<std::vec::Vec<pgorm::ExpectedColumn>> {
                    std::option::Option::Some(std::vec![
                        #(#expected_columns),*
                    ])
                }
            }
        )
    }

    fn impl_model_trait<'a>(&'a self) -> TokenStream {
        let ident = &self.ident;
        let entity_ident = &self.entity_ident;
        let ignore_attrs = &self.ignore_attrs;
        let ignore = |(ident, ignore): (&'a Ident, &bool)| -> Option<&'a Ident> {
            if *ignore { None } else { Some(ident) }
        };
        let field_idents: Vec<&Ident> = self
            .field_idents
            .iter()
            .zip(ignore_attrs)
            .filter_map(ignore)
            .collect();
        let column_idents: Vec<&Ident> = self
            .column_idents
            .iter()
            .zip(ignore_attrs)
            .filter_map(ignore)
            .collect();

        let missing_field_msg = format!("field does not exist on {ident}");
        let mismatch_msgs: Vec<String> = field_idents
            .iter()
            .map(|field| format!("value does not match the type of {ident} field {field}"))
            .collect();

        quote!(
            #[automatically_derived]
            impl pgorm::ModelTrait for #ident {
                type Entity = #entity_ident;

                fn get(&self, c: <Self::Entity as pgorm::entity::EntityTrait>::Column) -> pgorm::Value {
                    match c {
                        #(<Self::Entity as pgorm::entity::EntityTrait>::Column::#column_idents => self.#field_idents.clone().into(),)*
                        _ => panic!(#missing_field_msg),
                    }
                }

                fn set(&mut self, c: <Self::Entity as pgorm::entity::EntityTrait>::Column, v: pgorm::Value) -> std::result::Result<(), pgorm::Error> {
                    match c {
                        #(<Self::Entity as pgorm::entity::EntityTrait>::Column::#column_idents => {
                            self.#field_idents = pgorm::pgorm_query::ValueType::try_from(v)
                                .map_err(|_| pgorm::Error::Type(#mismatch_msgs.to_owned()))?;
                            Ok(())
                        },)*
                        _ => Err(pgorm::Error::Type(#missing_field_msg.to_owned())),
                    }
                }
            }
        )
    }
}

/// Method to derive an ActiveModel
// [spec:pgorm:sem:macros.derive.model+4]
// [spec:pgorm:syn:macros.derive.entity-model.attrs+6]    the field-key vocabulary this
// derive shares with the `Entity` derive
pub fn expand_derive_model(input: syn::DeriveInput) -> syn::Result<TokenStream> {
    let ident_span = input.ident.span();
    match Model::new(input) {
        Ok(model) => model.expand(),
        Err(Error::InputNotStruct) => Ok(quote_spanned! {
            ident_span => compile_error!("you can only derive `Model` on structs");
        }),
        Err(Error::Syn(err)) => Err(err),
    }
}
