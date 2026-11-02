use super::util::unknown_pgorm_key;
use heck::{ToLowerCamelCase, ToSnakeCase};
use proc_macro2::{Ident, TokenStream};
use quote::{quote, quote_spanned};
use syn::{Attribute, Data, DataEnum, Fields, LitStr, Variant};

/// Refuse every key on the enum itself: `Column` reads only a variant's
/// `column_name`.
// [spec:pgorm:syn:macros.derive.entity-model.attrs+7]
fn refuse_container_keys(attrs: &[Attribute]) -> syn::Result<()> {
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("pgorm")) {
        attr.parse_nested_meta(|meta| Err(unknown_pgorm_key(&meta)))?;
    }
    Ok(())
}

/// Derive a Column name for an enum type
pub fn impl_default_as_str(ident: &Ident, data: &Data) -> syn::Result<TokenStream> {
    let variants = match data {
        syn::Data::Enum(DataEnum { variants, .. }) => variants,
        _ => {
            return Ok(quote_spanned! {
                ident.span() => compile_error!("you can only derive `Column` on enums");
            });
        }
    };

    let variant: Vec<TokenStream> = variants
        .iter()
        .map(|Variant { ident, fields, .. }| match fields {
            Fields::Named(_) => quote! { #ident{..} },
            Fields::Unnamed(_) => quote! { #ident(..) },
            Fields::Unit => quote! { #ident },
        })
        .collect();

    let name: Vec<TokenStream> = variants
        .iter()
        .map(|v| {
            let mut column_name = v.ident.to_string().to_snake_case();
            for attr in v.attrs.iter() {
                if !attr.path().is_ident("pgorm") {
                    continue;
                }
                attr.parse_nested_meta(|meta| {
                    if !meta.path.is_ident("column_name") {
                        return Err(unknown_pgorm_key(&meta));
                    }
                    column_name = meta.value()?.parse::<LitStr>()?.value();
                    Ok(())
                })?;
            }
            Ok::<TokenStream, syn::Error>(quote! { #column_name })
        })
        .collect::<Result<_, _>>()?;

    Ok(quote!(
        #[automatically_derived]
        impl #ident {
            fn default_as_str(&self) -> &str {
                match self {
                    #(Self::#variant => #name),*
                }
            }
        }
    ))
}

/// Implement `FromStr` for a column enum under the `Column` derive.
pub fn impl_col_from_str(ident: &Ident, data: &Data) -> syn::Result<TokenStream> {
    let data_enum = match data {
        Data::Enum(data_enum) => data_enum,
        _ => {
            return Ok(quote_spanned! {
                ident.span() => compile_error!("you can only derive `Column` on enums");
            });
        }
    };

    let columns = data_enum.variants.iter().map(|column| {
        let column_iden = column.ident.clone();
        let column_str_snake = column_iden.to_string().to_snake_case();
        let column_str_mixed = column_iden.to_string().to_lower_camel_case();
        quote!(
            #column_str_snake | #column_str_mixed => Ok(#ident::#column_iden)
        )
    });

    Ok(quote!(
        #[automatically_derived]
        impl std::str::FromStr for #ident {
            type Err = pgorm::ColumnFromStrError;

            fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
                match s {
                    #(#columns),*,
                    _ => Err(pgorm::ColumnFromStrError(s.to_owned())),
                }
            }
        }
    ))
}

// [spec:pgorm:sem:macros.derive.column+6]
pub fn expand_derive_column(
    ident: &Ident,
    data: &Data,
    attrs: &[Attribute],
) -> syn::Result<TokenStream> {
    refuse_container_keys(attrs)?;
    let impl_default_as_str = impl_default_as_str(ident, data)?;
    let impl_col_from_str = impl_col_from_str(ident, data)?;

    Ok(quote!(
        #impl_default_as_str

        #impl_col_from_str

        #[automatically_derived]
        impl pgorm::SqlName for #ident {
            fn unquoted(&self, s: &mut dyn std::fmt::Write) {
                write!(s, "{}", pgorm::StaticName::as_str(self)).unwrap();
            }
        }

        #[automatically_derived]
        impl pgorm::StaticName for #ident {
            fn as_str(&self) -> &str {
                self.default_as_str()
            }
        }
    ))
}
