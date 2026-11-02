//! The expansion of the `SqlName` and `StaticName` derives.
//!
//! Two proc-macro crates define these derives and differ only in the path
//! their expansion names the traits by: `pgorm-query-derive` writes
//! `pgorm_query`, for a crate depending on pgorm-query, and `pgorm-macros`
//! writes `pgorm::pgorm_query`, for a crate depending only on pgorm. Each
//! passes its path as `root`; the path is call-site relative, as every
//! derive's in the workspace is, so it is never `::`-prefixed.

use std::convert::{TryFrom, TryInto};

use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    Attribute, DataEnum, DataStruct, DeriveInput, Fields, Variant, punctuated::Punctuated,
    token::Comma,
};

mod error;
mod iden_attr;
mod iden_path;
mod iden_variant;

use self::{
    error::ErrorMsg,
    iden_attr::IdenAttr,
    iden_path::IdenPath,
    iden_variant::{IdenVariant, SqlNameArm, StaticNameArm},
};

fn find_attr(attrs: &[Attribute]) -> Option<&Attribute> {
    attrs.iter().find(|attr| {
        attr.path().is_ident(&IdenPath::Iden) || attr.path().is_ident(&IdenPath::Method)
    })
}

fn get_table_name(ident: &proc_macro2::Ident, attrs: &[Attribute]) -> Result<String, syn::Error> {
    let table_name = match find_attr(attrs) {
        Some(att) => match att.try_into()? {
            IdenAttr::Rename(lit) => lit,
            _ => return Err(syn::Error::new_spanned(att, ErrorMsg::ContainerAttr)),
        },
        None => ident.to_string().to_snake_case(),
    };
    Ok(table_name)
}

fn must_be_valid_iden(name: &str) -> bool {
    name.chars()
        .take(1)
        .all(|c| c == '_' || c.is_ascii_alphabetic())
        && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn impl_sql_name_for_unit_struct(
    root: &TokenStream,
    ident: &proc_macro2::Ident,
    table_name: &str,
) -> TokenStream {
    let prepare = if must_be_valid_iden(table_name) {
        quote! {
            fn prepare(&self, s: &mut dyn ::std::fmt::Write) {
                write!(s, "\"").unwrap();
                self.unquoted(s);
                write!(s, "\"").unwrap();
            }
        }
    } else {
        quote! {}
    };

    quote! {
        impl #root::SqlName for #ident {
            #prepare

            fn unquoted(&self, s: &mut dyn ::std::fmt::Write) {
                write!(s, "{}", #table_name).unwrap();
            }
        }
    }
}

fn impl_sql_name_for_enum<'a, T>(
    root: &TokenStream,
    ident: &proc_macro2::Ident,
    table_name: &str,
    variants: T,
) -> syn::Result<TokenStream>
where
    T: Iterator<Item = &'a Variant>,
{
    let mut is_all_valid = true;

    let match_arms = variants
        .map(|v| (table_name, v))
        .map(|v| {
            let v = IdenVariant::<SqlNameArm>::try_from(v)?;
            is_all_valid &= v.must_be_valid_iden();
            Ok(v)
        })
        .collect::<syn::Result<Vec<_>>>()?;

    let prepare = if is_all_valid {
        quote! {
            fn prepare(&self, s: &mut dyn ::std::fmt::Write) {
                write!(s, "\"").unwrap();
                self.unquoted(s);
                write!(s, "\"").unwrap();
            }
        }
    } else {
        quote! {}
    };

    Ok(quote! {
        impl #root::SqlName for #ident {
            #prepare

            fn unquoted(&self, s: &mut dyn ::std::fmt::Write) {
                match self {
                    #(#match_arms),*
                };
            }
        }
    })
}

/// The variants of an enum, or `None` for a unit struct; any other shape is
/// refused at the type's name.
fn variants_of<'a>(
    ident: &proc_macro2::Ident,
    data: &'a syn::Data,
) -> syn::Result<Option<&'a Punctuated<Variant, Comma>>> {
    match data {
        syn::Data::Enum(DataEnum { variants, .. }) => Ok(Some(variants)),
        syn::Data::Struct(DataStruct {
            fields: Fields::Unit,
            ..
        }) => Ok(None),
        _ => Err(syn::Error::new(
            ident.span(),
            "you can only derive SqlName on enums or unit structs",
        )),
    }
}

/// Implement `SqlName` for an enum or unit struct, naming the trait through
/// `root`. An empty enum expands to nothing.
pub fn sql_name(input: &DeriveInput, root: &TokenStream) -> syn::Result<TokenStream> {
    let DeriveInput {
        ident, data, attrs, ..
    } = input;
    let table_name = get_table_name(ident, attrs)?;

    match variants_of(ident, data)? {
        None => Ok(impl_sql_name_for_unit_struct(root, ident, &table_name)),
        Some(variants) if variants.is_empty() => Ok(TokenStream::new()),
        Some(variants) => impl_sql_name_for_enum(root, ident, &table_name, variants.iter()),
    }
}

/// Implement `SqlName`, `StaticName` and `AsRef<str>` for an enum or unit
/// struct, naming the traits through `root`. An empty enum expands to nothing.
pub fn static_name(input: &DeriveInput, root: &TokenStream) -> syn::Result<TokenStream> {
    let DeriveInput {
        ident, data, attrs, ..
    } = input;
    let table_name = get_table_name(ident, attrs)?;

    let (impl_iden, as_str) = match variants_of(ident, data)? {
        None => (
            impl_sql_name_for_unit_struct(root, ident, &table_name),
            quote! { #table_name },
        ),
        Some(variants) if variants.is_empty() => return Ok(TokenStream::new()),
        Some(variants) => {
            let impl_iden = impl_sql_name_for_enum(root, ident, &table_name, variants.iter())?;
            let match_arms = variants
                .iter()
                .map(|v| (table_name.as_str(), v))
                .map(IdenVariant::<StaticNameArm>::try_from)
                .collect::<syn::Result<Vec<_>>>()?;
            (
                impl_iden,
                quote! {
                    match self {
                        #(#match_arms),*
                    }
                },
            )
        }
    };

    Ok(quote! {
        #impl_iden

        impl #root::StaticName for #ident {
            fn as_str(&self) -> &str {
                #as_str
            }
        }

        impl ::std::convert::AsRef<str> for #ident {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
    })
}
