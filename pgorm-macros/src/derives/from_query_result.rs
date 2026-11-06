use super::util::{PROJECTION_FIELD_KEYS, skip_known_key, spell_type, unknown_pgorm_key};
use proc_macro2::{Ident, TokenStream};
use quote::{ToTokens, format_ident, quote, quote_spanned};
use syn::{
    Attribute, Data, DataStruct, Fields, Generics, Type, ext::IdentExt, meta::ParseNestedMeta,
};

pub struct FromQueryResultItem {
    pub skip: bool,
    pub ident: Ident,
    pub ty: Type,
    /// The entity column a `PartialModel` field reads, whose read cast its
    /// reported column carries.
    pub column: Option<TokenStream>,
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
    // [spec:pgorm:sem:macros.derive.from-query-result+4]    the reflected column
    fn expected_column(&self) -> Option<TokenStream> {
        if self.skip {
            return None;
        }

        let Self {
            ident, ty, column, ..
        } = self;
        let name = ident.unraw().to_string();
        let rust_type = spell_type(ty);
        let selected_as = column.as_ref().map(|column| quote!(.selected_as(&#column)));

        Some(quote! {
            pgorm::ExpectedColumn::new(#name, #rust_type, <#ty as pgorm::TryGetable>::accepts)
                #selected_as
        })
    }
}

/// Which derive is writing `FromQueryResult`: its own, or `PartialModel`,
/// which writes it beside the projection and reads the keys that say what
/// is projected, handing over the entity column each field reads, in field
/// order, or `None` for a field projecting an expression.
#[derive(Clone, Copy)]
pub(crate) enum Decoder<'a> {
    FromQueryResult,
    PartialModel(&'a [Option<TokenStream>]),
}

/// The refusal for a key `FromQueryResult` does not read: one `PartialModel`
/// reads names that derive, which writes `FromQueryResult` itself.
// [spec:pgorm:sem:macros.derive.from-query-result+4]
fn unread_key(meta: &ParseNestedMeta<'_>) -> syn::Error {
    match meta.path.get_ident() {
        Some(key)
            if key == "entity" || PROJECTION_FIELD_KEYS.contains(&key.to_string().as_str()) =>
        {
            meta.error(format!(
                "`{key}` is read by `PartialModel`, which implements `FromQueryResult` too: \
             derive `PartialModel` in its place"
            ))
        }
        _ => unknown_pgorm_key(meta),
    }
}

/// Refuse every container key: `FromQueryResult` reads none.
fn refuse_container_keys(attrs: &[Attribute]) -> syn::Result<()> {
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("pgorm")) {
        attr.parse_nested_meta(|meta| Err(unread_key(&meta)))?;
    }
    Ok(())
}

/// Method to derive a [QueryResult](pgorm::QueryResult)
// [spec:pgorm:sem:macros.derive.from-query-result+4]
pub fn expand_derive_from_query_result(
    ident: Ident,
    data: Data,
    generics: Generics,
    attrs: &[Attribute],
) -> syn::Result<TokenStream> {
    refuse_container_keys(attrs)?;
    expand_from_query_result(ident, data, generics, Decoder::FromQueryResult)
}

/// `FromQueryResult` as `decoder` writes it. Under `PartialModel`, which
/// reads the container and the projection keys itself, a field's
/// `from_col` and `from_expr` are stepped over.
// [spec:pgorm:sem:macros.derive.from-query-result+4]
pub(crate) fn expand_from_query_result(
    ident: Ident,
    data: Data,
    generics: Generics,
    decoder: Decoder<'_>,
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
                } else if matches!(decoder, Decoder::PartialModel(_)) {
                    skip_known_key(&meta, &PROJECTION_FIELD_KEYS)
                } else {
                    Err(unread_key(&meta))
                }
            })?;
        }
        let ty = parsed_field.ty;
        let ident = format_ident!("{}", parsed_field.ident.unwrap().to_string());
        let column = match decoder {
            Decoder::PartialModel(columns) => columns.get(field.len()).cloned().flatten(),
            Decoder::FromQueryResult => None,
        };
        field.push(FromQueryResultItem {
            skip,
            ident,
            ty,
            column,
        });
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
