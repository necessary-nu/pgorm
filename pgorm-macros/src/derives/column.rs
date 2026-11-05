use super::iterable::impl_iterable;
use super::util::unknown_pgorm_key;
use heck::{ToLowerCamelCase, ToSnakeCase};
use proc_macro2::{Ident, TokenStream};
use quote::{quote, quote_spanned};
use syn::{Attribute, Data, DataEnum, Fields, LitStr, Variant, Visibility};

/// One variant of a column enum: the variant, the column's SQL name, and
/// whether iterating the enum yields it.
pub(crate) struct ColumnVariant {
    pub(crate) variant: Variant,
    pub(crate) name: String,
    pub(crate) iterated: bool,
}

/// Refuse every key on the enum itself: `Column` reads only a variant's
/// `column_name`.
// [spec:pgorm:syn:macros.derive.entity-model.attrs+10]
fn refuse_container_keys(attrs: &[Attribute]) -> syn::Result<()> {
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("pgorm")) {
        attr.parse_nested_meta(|meta| Err(unknown_pgorm_key(&meta)))?;
    }
    Ok(())
}

/// The column a variant of a program's column enum names: the snake_case of
/// its name unless `column_name` says otherwise.
fn column_variant(variant: &Variant) -> syn::Result<ColumnVariant> {
    let mut name = variant.ident.to_string().to_snake_case();
    for attr in variant
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("pgorm"))
    {
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("column_name") {
                return Err(unknown_pgorm_key(&meta));
            }
            name = meta.value()?.parse::<LitStr>()?.value();
            Ok(())
        })?;
    }
    Ok(ColumnVariant {
        variant: variant.clone(),
        name,
        iterated: true,
    })
}

/// What makes `ident` a column enum: each variant's SQL name through
/// `SqlName` and `StaticName`, `FromStr` from the name, and iteration over
/// the variants marked iterated. The `Column` derive writes it for a
/// program's enum, and the composite derive for the `Column` it makes.
// [spec:pgorm:sem:macros.derive.column+7]
// [spec:pgorm:sem:macros.derive.boilerplate]
pub(crate) fn column_impls(
    ident: &Ident,
    vis: &Visibility,
    variants: &[ColumnVariant],
) -> TokenStream {
    let pattern = variants.iter().map(|ColumnVariant { variant, .. }| {
        let ident = &variant.ident;
        match variant.fields {
            Fields::Named(_) => quote! { #ident{..} },
            Fields::Unnamed(_) => quote! { #ident(..) },
            Fields::Unit => quote! { #ident },
        }
    });
    let name = variants.iter().map(|ColumnVariant { name, .. }| name);

    let from_str = variants.iter().map(|ColumnVariant { variant, .. }| {
        let column_iden = &variant.ident;
        let column_str_snake = column_iden.to_string().to_snake_case();
        let column_str_mixed = column_iden.to_string().to_lower_camel_case();
        quote!(
            #column_str_snake | #column_str_mixed => Ok(#ident::#column_iden)
        )
    });

    let iterable = impl_iterable(
        ident,
        vis,
        variants
            .iter()
            .filter(|column| column.iterated)
            .map(|column| &column.variant),
    );

    quote!(
        #[automatically_derived]
        impl #ident {
            fn default_as_str(&self) -> &str {
                match self {
                    #(Self::#pattern => #name),*
                }
            }
        }

        #[automatically_derived]
        impl std::str::FromStr for #ident {
            type Err = pgorm::ColumnFromStrError;

            fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
                match s {
                    #(#from_str),*,
                    _ => Err(pgorm::ColumnFromStrError(s.to_owned())),
                }
            }
        }

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

        #iterable
    )
}

// [spec:pgorm:sem:macros.derive.column+7]
pub fn expand_derive_column(input: &syn::DeriveInput) -> syn::Result<TokenStream> {
    let ident = &input.ident;
    let Data::Enum(DataEnum { variants, .. }) = &input.data else {
        return Ok(quote_spanned! {
            ident.span() => compile_error!("you can only derive `Column` on enums");
        });
    };
    refuse_container_keys(&input.attrs)?;
    let variants = variants
        .iter()
        .map(column_variant)
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(column_impls(ident, &input.vis, &variants))
}
