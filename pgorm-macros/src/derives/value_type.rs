use super::sql_type_match::{arr_type_match, col_type_match, unwrap_option};
use super::util::bare_code;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, Type, spanned::Spanned};

// [spec:pgorm:sem:macros.derive.value-type+6]
// [spec:pgorm:req:macros.derive.code-values+3]
pub fn expand_derive_value_type(input: syn::DeriveInput) -> syn::Result<TokenStream> {
    let syn::DeriveInput {
        ident: name,
        data,
        attrs,
        ..
    } = input;

    let syn::Data::Struct(syn::DataStruct {
        fields: syn::Fields::Unnamed(syn::FieldsUnnamed { unnamed, .. }),
        ..
    }) = data
    else {
        return Err(syn::Error::new(
            name.span(),
            "`ValueType` can only be derived on a tuple struct",
        ));
    };

    let Some(field) = unnamed.into_iter().next() else {
        return Err(syn::Error::new(
            name.span(),
            "`ValueType` requires the tuple struct to hold a value field",
        ));
    };

    let mut col_type = None;
    let mut arr_type = None;

    for attr in attrs.iter() {
        if !attr.path().is_ident("pgorm") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("column_type") {
                col_type = Some(bare_code(&meta, "column_type")?);
            } else if meta.path.is_ident("array_type") {
                arr_type = Some(bare_code(&meta, "array_type")?);
            } else {
                return Err(meta.error("Invalid attribute: expected `column_type` or `array_type`"));
            }

            Ok(())
        })?;
    }

    let field_span = field.span();
    let ty = field.ty;
    let inner_type = unwrap_option(&ty).unwrap_or(&ty);

    Ok(impl_value_type(
        &name,
        &ty,
        &col_type_match(col_type, inner_type, field_span)?,
        &arr_type_match(arr_type, inner_type, field_span),
    ))
}

fn impl_value_type(
    name: &Ident,
    field_type: &Type,
    column_type: &TokenStream,
    array_type: &TokenStream,
) -> TokenStream {
    quote!(
        #[automatically_derived]
        impl std::convert::From<#name> for pgorm::Value {
            fn from(source: #name) -> Self {
                source.0.into()
            }
        }

        #[automatically_derived]
        impl pgorm::TryGetable for #name {
            fn try_get_by<I: pgorm::RowIndex + std::fmt::Display>(res: &pgorm::QueryResult, idx: I)
                -> std::result::Result<Self, pgorm::TryGetError> {
                <#field_type as pgorm::TryGetable>::try_get_by(res, idx).map(|v| #name(v))
            }
        }

        #[automatically_derived]
        impl pgorm::pgorm_query::ValueType for #name {
            fn from_value(v: pgorm::Value) -> std::result::Result<Self, pgorm::pgorm_query::ValueTypeError> {
                <#field_type as pgorm::pgorm_query::ValueType>::from_value(v).map(|v| #name(v))
            }

            fn type_name() -> std::string::String {
                stringify!(#name).to_owned()
            }

            fn array_type() -> pgorm::pgorm_query::ArrayType {
                #array_type
            }

            fn column_type() -> pgorm::pgorm_query::ColumnType {
                #column_type
            }
        }
    )
}
