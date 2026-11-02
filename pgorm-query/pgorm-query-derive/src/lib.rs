//! The `SqlName` and `StaticName` derives for a crate depending on
//! pgorm-query. Their expansion names the traits `pgorm_query::..`; pgorm
//! re-exports pgorm-macros' derives of the same names, which share this
//! implementation and name them through `pgorm::pgorm_query`.

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

// [spec:pgorm:sem:macros.derive.iden.query+1]
#[doc(alias = "Iden")]
#[proc_macro_derive(SqlName, attributes(iden, method))]
pub fn derive_sql_name(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    pgorm_query_derive_internals::sql_name(&input, &quote!(pgorm_query))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

// [spec:pgorm:sem:macros.derive.iden.query+1]
#[proc_macro_derive(StaticName, attributes(iden, method))]
pub fn derive_static_name(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    pgorm_query_derive_internals::static_name(&input, &quote!(pgorm_query))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
