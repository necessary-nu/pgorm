use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned};

use super::util::{bare_code, unknown_pgorm_key};

/// The active model a form converts into, from the container's
/// `active_model`, the one key the derive reads. Every field becomes its
/// active model's field of the same name, so a field carries no key.
// [spec:pgorm:sem:macros.derive.active-model+9]
// [spec:pgorm:req:macros.derive.code-values+4]
fn active_model_name(input: &syn::DeriveInput) -> syn::Result<Option<syn::Path>> {
    let mut active_model = None;
    for attr in input
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("pgorm"))
    {
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("active_model") {
                return Err(unknown_pgorm_key(&meta));
            }
            if active_model.is_some() {
                return Err(meta.error("duplicate `active_model`"));
            }
            active_model = Some(bare_code(&meta, "active_model")?);
            Ok(())
        })?;
    }
    if let syn::Data::Struct(data) = &input.data {
        for field in data.fields.iter() {
            for attr in field
                .attrs
                .iter()
                .filter(|attr| attr.path().is_ident("pgorm"))
            {
                attr.parse_nested_meta(|meta| Err(unknown_pgorm_key(&meta)))?;
            }
        }
    }
    Ok(active_model)
}

enum Error {
    InputNotStruct,
    Syn(syn::Error),
}

struct IntoActiveModel {
    active_model: Option<syn::Path>,
    fields: syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
    field_idents: Vec<syn::Ident>,
    ident: syn::Ident,
}

impl IntoActiveModel {
    fn new(input: syn::DeriveInput) -> Result<Self, Error> {
        let active_model = active_model_name(&input).map_err(Error::Syn)?;
        let fields = match input.data {
            syn::Data::Struct(syn::DataStruct {
                fields: syn::Fields::Named(syn::FieldsNamed { named, .. }),
                ..
            }) => named,
            _ => return Err(Error::InputNotStruct),
        };

        let ident = input.ident;

        let field_idents = fields
            .iter()
            .map(|field| field.ident.as_ref().unwrap().clone())
            .collect();

        Ok(IntoActiveModel {
            active_model,
            fields,
            field_idents,
            ident,
        })
    }

    fn expand(&self) -> syn::Result<TokenStream> {
        let expanded_impl_into_active_model = self.impl_into_active_model();

        Ok(expanded_impl_into_active_model)
    }

    fn impl_into_active_model(&self) -> TokenStream {
        let Self {
            active_model,
            ident,
            field_idents,
            fields,
        } = self;

        let active_model_ident = active_model
            .clone()
            .unwrap_or_else(|| syn::Ident::new("ActiveModel", Span::call_site()).into());

        let expanded_fields_into_active_model = fields.iter().map(|field| {
            let field_ident = field.ident.as_ref().unwrap();

            quote!(
                pgorm::IntoActiveValue::<_>::into_active_value(self.#field_ident).into()
            )
        });

        quote!(
            #[automatically_derived]
            impl pgorm::IntoActiveModel<#active_model_ident> for #ident {
                fn into_active_model(self) -> #active_model_ident {
                    #active_model_ident {
                        #( #field_idents: #expanded_fields_into_active_model, )*
                        ..::std::default::Default::default()
                    }
                }
            }
        )
    }
}

/// Method to derive the ActiveModel from the [ActiveModelTrait](pgorm::ActiveModelTrait)
// [spec:pgorm:sem:macros.derive.active-model+9]
pub fn expand_into_active_model(input: syn::DeriveInput) -> syn::Result<TokenStream> {
    let ident_span = input.ident.span();

    match IntoActiveModel::new(input) {
        Ok(model) => model.expand(),
        Err(Error::InputNotStruct) => Ok(quote_spanned! {
            ident_span => compile_error!("you can only derive `IntoActiveModel` on structs");
        }),
        Err(Error::Syn(err)) => Err(err),
    }
}
