use super::util::{PROJECTION_FIELD_KEYS, skip_known_key, spell_type};
use proc_macro2::{Ident, TokenStream};
use quote::{ToTokens, format_ident, quote, quote_spanned};
use syn::{Data, DataStruct, Fields, Generics, Type, ext::IdentExt};

pub struct FromQueryResultItem {
    pub skip: bool,
    pub ident: Ident,
    pub ty: Type,
}
impl ToTokens for FromQueryResultItem {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self { ident, skip, .. } = self;
        if *skip {
            tokens.extend(quote! {
                #ident: std::default::Default::default(),
            });
        } else {
            let name = ident.unraw().to_string();
            tokens.extend(quote! {
                #ident: row.try_get(pre, #name)?,
            });
        }
    }
}

impl FromQueryResultItem {
    /// The column this field reads, as an `ExpectedColumn`, or `None` for a
    /// skipped field, which reads no column at all.
    // [spec:pgorm:sem:macros.derive.from-query-result+3]    the reflected column
    fn expected_column(&self) -> Option<TokenStream> {
        if self.skip {
            return None;
        }

        let Self { ident, ty, .. } = self;
        let name = ident.unraw().to_string();
        let rust_type = spell_type(ty);

        Some(quote! {
            pgorm::ExpectedColumn::new(#name, #rust_type, <#ty as pgorm::TryGetable>::accepts)
        })
    }
}

/// Method to derive a [QueryResult](pgorm::QueryResult)
// [spec:pgorm:sem:macros.derive.from-query-result+3]
pub fn expand_derive_from_query_result(
    ident: Ident,
    data: Data,
    generics: Generics,
) -> syn::Result<TokenStream> {
    let fields = match data {
        Data::Struct(DataStruct {
            fields: Fields::Named(named),
            ..
        }) => named.named,
        _ => {
            return Ok(quote_spanned! {
                ident.span() => compile_error!("you can only derive FromQueryResult on structs");
            });
        }
    };
    let mut field = Vec::with_capacity(fields.len());

    for parsed_field in fields.into_iter() {
        // `skip` accumulates across every meta item of every `#[pgorm(...)]` attribute on
        // the field: once asked for, it cannot be un-asked by a later key.
        let mut skip = false;
        for attr in parsed_field.attrs.iter() {
            if !attr.path().is_ident("pgorm") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("skip") {
                    skip = true;
                    Ok(())
                } else {
                    skip_known_key(&meta, &PROJECTION_FIELD_KEYS)
                }
            })?;
        }
        let ty = parsed_field.ty;
        let ident = format_ident!("{}", parsed_field.ident.unwrap().to_string());
        field.push(FromQueryResultItem { skip, ident, ty });
    }
    let expected: Vec<TokenStream> = field
        .iter()
        .filter_map(FromQueryResultItem::expected_column)
        .collect();
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    Ok(quote!(
        #[automatically_derived]
        impl #impl_generics pgorm::FromQueryResult for #ident #ty_generics #where_clause {
            fn from_query_result(row: &pgorm::QueryResult, pre: &str) -> std::result::Result<Self, pgorm::Error> {
                Ok(Self {
                    #(#field)*
                })
            }

            fn expected_columns() -> std::option::Option<std::vec::Vec<pgorm::ExpectedColumn>> {
                std::option::Option::Some(std::vec![
                    #(#expected),*
                ])
            }
        }
    ))
}
