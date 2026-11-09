use super::util::into_active_value;
use proc_macro2::{Ident, TokenStream};
use quote::quote;

// [spec:pgorm:sem:macros.derive.from-query-result+4]
pub fn expand_derive_json_value_type(ident: Ident) -> syn::Result<TokenStream> {
    let into_active_value = into_active_value(&ident);
    let impl_not_u8 = quote!(
        #[automatically_derived]
        impl pgorm::pgorm_query::value::with_array::NotU8 for #ident {}
    );

    Ok(quote!(
        #[automatically_derived]
        impl pgorm::TryGetableFromJson for #ident {}

        #[automatically_derived]
        impl std::convert::From<#ident> for pgorm::Value {
            // `From` cannot report failure, and the alternative to panicking is binding
            // SQL NULL for a value that was never null: a confusing constraint violation
            // on a NOT NULL column, and silent data loss on a nullable one.
            #[allow(clippy::panic)]
            fn from(source: #ident) -> Self {
                match serde_json::to_value(&source) {
                    Ok(json) => pgorm::Value::Json(Some(std::boxed::Box::new(json))),
                    Err(err) => panic!(
                        "`{}` could not be serialized to JSON and cannot be bound as a value: {}",
                        stringify!(#ident),
                        err,
                    ),
                }
            }
        }

        #[automatically_derived]
        impl pgorm::pgorm_query::ValueType for #ident {
            fn from_value(v: pgorm::Value) -> Result<Self, pgorm::pgorm_query::ValueTypeError> {
                match v {
                    pgorm::Value::Json(Some(json)) => Ok(
                        serde_json::from_value(*json).map_err(|_| pgorm::pgorm_query::ValueTypeError)?,
                    ),
                    _ => Err(pgorm::pgorm_query::ValueTypeError),
                }
            }

            fn type_name() -> String {
                stringify!(#ident).to_owned()
            }

            fn array_type() -> pgorm::pgorm_query::ArrayType {
                pgorm::pgorm_query::ArrayType::Json
            }

            fn column_type() -> pgorm::pgorm_query::ColumnType {
                pgorm::pgorm_query::ColumnType::Json
            }
        }

        #[automatically_derived]
        impl pgorm::pgorm_query::Nullable for #ident {
            fn null() -> pgorm::Value {
                pgorm::Value::Json(None)
            }
        }

        #impl_not_u8

        #into_active_value
    ))
}
