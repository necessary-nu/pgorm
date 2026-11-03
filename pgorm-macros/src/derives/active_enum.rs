use super::case_style::{CaseStyle, convert_case_str};
use super::iterable::impl_iterable;
use super::util::{
    bare_code, camel_case_with_escaped_non_uax31, trim_starting_raw_identifier, unknown_pgorm_key,
};
use heck::ToSnakeCase;
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::{Expr, Lit, LitInt, LitStr, Type, UnOp, parse, spanned::Spanned};

enum Error {
    InputNotEnum,
    Syn(syn::Error),
    TT(TokenStream),
}

struct ActiveEnum {
    ident: syn::Ident,
    vis: syn::Visibility,
    enum_name: String,
    rs_type: Type,
    /// The column type the values are stored as, the expression `db_type`
    /// wrote, or else the PostgreSQL enum of the string values.
    db_type: TokenStream,
    is_string: bool,
    variants: Vec<ActiveEnumVariant>,
    display: bool,
}

struct ActiveEnumVariant {
    variant: syn::Variant,
    value: Value,
    display_value: Option<LitStr>,
}

/// The value a variant is stored as.
enum Value {
    String(String),
    Number(LitInt),
}

/// A variant's own keys.
struct VariantKeys {
    string_value: Option<LitStr>,
    num_value: Option<LitInt>,
    rename: Option<CaseStyle>,
    display_value: Option<LitStr>,
}

impl VariantKeys {
    /// The keys on `variant`, any other being refused at the key; the
    /// `display_value` that only `display` reads is refused without it.
    // [spec:pgorm:syn:macros.derive.active-enum+6]
    fn parse(variant: &syn::Variant, display: bool) -> syn::Result<Self> {
        let mut keys = Self {
            string_value: None,
            num_value: None,
            rename: None,
            display_value: None,
        };
        for attr in variant
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("pgorm"))
        {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("string_value") {
                    keys.string_value = Some(meta.value()?.parse::<LitStr>()?);
                } else if meta.path.is_ident("num_value") {
                    keys.num_value = Some(meta.value()?.parse::<LitInt>()?);
                } else if meta.path.is_ident("display_value") {
                    let value = meta.value()?.parse::<LitStr>()?;
                    // An opt-in `Display` is what reads it, so without one
                    // the text would be dropped unread.
                    if !display {
                        return Err(syn::Error::new_spanned(
                            &value,
                            "`display_value` is the text the `display` key writes: add \
                             `display` to the enum's `#[pgorm(..)]`",
                        ));
                    }
                    keys.display_value = Some(value);
                } else if meta.path.is_ident("rename") {
                    keys.rename = Some((&meta).try_into()?);
                } else {
                    return Err(unknown_pgorm_key(&meta));
                }
                Ok(())
            })?;
        }
        Ok(keys)
    }
}

/// The most bytes PostgreSQL holds in an enum's value, `NAMEDATALEN - 1`:
/// `CREATE TYPE` refuses a longer one (`42602`), and an empty one is a value.
const ENUM_LABEL_BYTES: usize = 63;

/// Refuse a PostgreSQL enum's value longer than the server holds, at `span`,
/// the value's literal or the variant it is derived from; a disjoint entity's
/// kind type is expanded as an `ActiveEnum`, so its values are checked here
/// too.
// [spec:pgorm:syn:macros.derive.active-enum+6]
fn check_enum_label(label: &str, span: Span) -> syn::Result<()> {
    if label.len() > ENUM_LABEL_BYTES {
        return Err(syn::Error::new(
            span,
            format!(
                "`{label}` is {} bytes, and a PostgreSQL enum's value is at most \
                 {ENUM_LABEL_BYTES}",
                label.len()
            ),
        ));
    }
    Ok(())
}

/// The integer a variant's discriminant writes, a negative literal included.
fn discriminant(variant: &syn::Variant) -> Result<Option<LitInt>, Error> {
    let span = variant.ident.span();
    match &variant.discriminant {
        None => Ok(None),
        Some((_, Expr::Lit(exprlit))) => match &exprlit.lit {
            Lit::Int(litint) => Ok(Some(litint.clone())),
            _ => Err(Error::TT(quote_spanned! {
                span => compile_error!("Enum variant discriminant is not an integer");
            })),
        },
        // Rust writes a negative discriminant as a unary minus on a literal.
        Some((_, Expr::Unary(unary))) => match (&unary.op, &*unary.expr) {
            (UnOp::Neg(_), Expr::Lit(exprlit)) => match &exprlit.lit {
                Lit::Int(litint) => {
                    let negative = quote! { -#litint };
                    Ok(Some(parse(negative.into()).map_err(Error::Syn)?))
                }
                _ => Err(Error::TT(quote_spanned! {
                    span => compile_error!("Enum variant discriminant is not an integer");
                })),
            },
            (UnOp::Neg(_), _) => Ok(None),
            _ => Err(Error::TT(quote_spanned! {
                span => compile_error!("Only - token is supported in enum variants, not ! and *");
            })),
        },
        Some(_) => Ok(None),
    }
}

impl ActiveEnum {
    /// An enum with neither `rs_type` nor `db_type` is a PostgreSQL enum of
    /// strings, each value its variant's `string_value` or else its name in
    /// the `rename` or `rename_all` style, snake_case by default, and its
    /// type the enum's name in snake_case unless `enum_name` says otherwise.
    /// An enum stored any other way names both keys, and its values are
    /// what its variants write, falling back to their discriminants.
    // [spec:pgorm:syn:macros.derive.active-enum+6]
    // [spec:pgorm:sem:macros.derive.boilerplate]
    // [spec:pgorm:req:macros.derive.code-values+2]
    fn new(input: syn::DeriveInput) -> Result<Self, Error> {
        let ident_span = input.ident.span();
        let ident = input.ident;

        let mut enum_name = None;
        let mut rs_type: Option<(Type, Span)> = None;
        let mut db_type: Option<Expr> = None;
        let mut rename_all = None;
        let mut schema_name: Option<String> = None;
        let mut display = false;

        input
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("pgorm"))
            .try_for_each(|attr| {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("rs_type") {
                        let span = meta.path.span();
                        rs_type = Some((bare_code(&meta, "rs_type")?, span));
                    } else if meta.path.is_ident("db_type") {
                        db_type = Some(bare_code(&meta, "db_type")?);
                    } else if meta.path.is_ident("schema_name") {
                        let litstr: LitStr = meta.value()?.parse()?;
                        schema_name = Some(litstr.value());
                    } else if meta.path.is_ident("enum_name") {
                        let litstr: LitStr = meta.value()?.parse()?;
                        enum_name = Some(litstr.value());
                    } else if meta.path.is_ident("rename_all") {
                        rename_all = Some((&meta).try_into()?);
                    } else if meta.path.is_ident("display") {
                        // [spec:pgorm:req:macros.derive.names+1]
                        display = true;
                    } else {
                        return Err(unknown_pgorm_key(&meta));
                    }
                    Ok(())
                })
                .map_err(Error::Syn)
            })?;

        let variant_vec = match input.data {
            syn::Data::Enum(syn::DataEnum { variants, .. }) => variants,
            _ => return Err(Error::InputNotEnum),
        };

        let enum_name =
            enum_name.unwrap_or_else(|| trim_starting_raw_identifier(&ident).to_snake_case());

        let (rs_type, db_type) = match (rs_type, db_type) {
            (None, None) => (None, None),
            (Some((rs_type, _)), Some(db_type)) => (Some(rs_type), Some(db_type)),
            (Some((_, span)), None) => {
                return Err(Error::Syn(syn::Error::new(
                    span,
                    "an enum stored as `rs_type` names the column type too, `db_type = \
                     ColumnType::..`; a PostgreSQL enum of strings names neither",
                )));
            }
            (None, Some(_)) => {
                return Err(Error::TT(quote_spanned! {
                    ident_span => compile_error!("Missing macro attribute `rs_type`");
                }));
            }
        };
        let postgres_enum = db_type.is_none();

        let mut is_string = postgres_enum || rename_all.is_some();
        let mut is_int = false;
        let mut variants = Vec::new();

        for variant in variant_vec {
            let keys = VariantKeys::parse(&variant, display).map_err(Error::Syn)?;
            if let Some(num_value) = &keys.num_value {
                if postgres_enum {
                    return Err(Error::Syn(syn::Error::new_spanned(
                        num_value,
                        "a PostgreSQL enum's values are strings; an enum stored as numbers \
                         names `rs_type` and `db_type`",
                    )));
                }
                is_int = true;
            }
            is_string |= keys.string_value.is_some() || keys.rename.is_some();
            let value = if let Some(string_value) = &keys.string_value {
                Some(Value::String(string_value.value()))
            } else if let Some(num_value) = &keys.num_value {
                Some(Value::Number(num_value.clone()))
            } else if let Some(rename) = keys.rename.or(rename_all) {
                Some(Value::String(convert_case_str(
                    &trim_starting_raw_identifier(&variant.ident),
                    Some(rename),
                )))
            } else if postgres_enum {
                if let Some((_, discriminant)) = &variant.discriminant {
                    return Err(Error::Syn(syn::Error::new_spanned(
                        discriminant,
                        "a PostgreSQL enum's values are its variants' names; an enum stored \
                         as its discriminants names `rs_type` and `db_type`",
                    )));
                }
                Some(Value::String(
                    trim_starting_raw_identifier(&variant.ident).to_snake_case(),
                ))
            } else {
                let number = discriminant(&variant)?;
                is_int |= number.is_some();
                number.map(Value::Number)
            };

            if is_string && is_int {
                return Err(Error::TT(quote_spanned! {
                    ident_span => compile_error!("All enum variants should specify the same `*_value` macro attribute, either `string_value` or `num_value` but not both");
                }));
            }

            let Some(value) = value else {
                let variant_span = variant.ident.span();
                return Err(Error::TT(quote_spanned! {
                    variant_span => compile_error!("Missing macro attribute, either `string_value`, `num_value` or `rename` should be specified or specify repr[X] and have a value for every entry");
                }));
            };
            if postgres_enum && let Value::String(label) = &value {
                let span = keys
                    .string_value
                    .as_ref()
                    .map_or_else(|| variant.ident.span(), LitStr::span);
                check_enum_label(label, span).map_err(Error::Syn)?;
            }

            variants.push(ActiveEnumVariant {
                variant,
                value,
                display_value: keys.display_value,
            });
        }

        let db_type = match db_type {
            Some(db_type) => quote! { #db_type },
            None => {
                let schema = match &schema_name {
                    Some(schema) => quote! {
                        Some(pgorm::pgorm_query::Name::runtime(#schema))
                    },
                    None => quote! { None },
                };
                quote! {
                    pgorm::ColumnType::Enum {
                        name: Self::name(),
                        schema: #schema,
                        variants: Self::name_values(),
                    }
                }
            }
        };

        Ok(ActiveEnum {
            ident,
            vis: input.vis,
            enum_name,
            rs_type: rs_type.unwrap_or_else(|| syn::parse_quote!(String)),
            db_type,
            is_string,
            variants,
            display,
        })
    }

    // [spec:pgorm:sem:macros.derive.active-enum.expansion+5]
    fn expand(&self) -> syn::Result<TokenStream> {
        let mut expanded = self.impl_active_enum();
        expanded.extend(impl_iterable(
            &self.ident,
            &self.vis,
            self.variants.iter().map(|variant| &variant.variant),
        ));
        if self.display {
            expanded.extend(self.impl_display());
        }

        Ok(expanded)
    }

    // [spec:pgorm:sem:macros.derive.active-enum.expansion+5]
    fn impl_display(&self) -> TokenStream {
        let ident = &self.ident;
        let variant_idents = self.variants.iter().map(|variant| &variant.variant.ident);
        let variant_display = self
            .variants
            .iter()
            .map(|variant| match &variant.display_value {
                Some(value) => value.value(),
                None => variant.variant.ident.to_string(),
            });

        quote!(
            #[automatically_derived]
            impl std::fmt::Display for #ident {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str(match self {
                        #( Self::#variant_idents => #variant_display, )*
                    })
                }
            }
        )
    }

    fn impl_active_enum(&self) -> TokenStream {
        let Self {
            ident,
            enum_name,
            rs_type,
            db_type,
            is_string,
            variants,
            ..
        } = self;

        let variant_idents: Vec<&syn::Ident> = variants
            .iter()
            .map(|variant| &variant.variant.ident)
            .collect();

        let variant_values: Vec<TokenStream> = variants
            .iter()
            .map(|variant| match &variant.value {
                Value::String(string) => quote! { #string },
                Value::Number(number) => quote! { #number },
            })
            .collect();

        let val = if *is_string {
            quote! { v.as_ref() }
        } else {
            quote! { v }
        };

        let enum_name_iden = format_ident!("{}Enum", ident);

        let str_variants: Vec<&String> = variants
            .iter()
            .filter_map(|variant| match &variant.value {
                Value::String(string) => Some(string),
                Value::Number(_) => None,
            })
            .collect();

        let impl_enum_variant_iden = if !str_variants.is_empty() {
            let enum_variant_iden = format_ident!("{}Variant", ident);
            let enum_variants: Vec<syn::Ident> = str_variants
                .iter()
                .map(|v| {
                    let v_cleaned = camel_case_with_escaped_non_uax31(v);

                    format_ident!("{}", v_cleaned)
                })
                .collect();

            quote!(
                #[doc = " Generated by pgorm-macros"]
                #[derive(Debug, Clone, PartialEq, Eq)]
                pub enum #enum_variant_iden {
                    #(
                        #[doc = " Generated by pgorm-macros"]
                        #enum_variants,
                    )*
                }

                #[automatically_derived]
                impl pgorm::pgorm_query::SqlName for #enum_variant_iden {
                    fn unquoted(&self, s: &mut dyn std::fmt::Write) {
                        write!(s, "{}", match self {
                            #(
                                Self::#enum_variants => #str_variants,
                            )*
                        }).unwrap();
                    }
                }

                #[automatically_derived]
                impl #ident {
                    #[doc = " Generated by pgorm-macros"]
                    pub fn name_values() -> Vec<pgorm::pgorm_query::Name> {
                        vec![
                            #( pgorm::pgorm_query::Name::new(#enum_variant_iden::#enum_variants), )*
                        ]
                    }
                }
            )
        } else {
            quote!()
        };

        let impl_not_u8 = quote!(
            #[automatically_derived]
            impl pgorm::pgorm_query::value::with_array::NotU8 for #ident {}
        );

        let impl_try_getable_array = quote!(
            #[automatically_derived]
            impl pgorm::TryGetableArray for #ident {
                fn try_get_by<I: pgorm::RowIndex + std::fmt::Display>(res: &pgorm::QueryResult, index: I) -> std::result::Result<Vec<Self>, pgorm::TryGetError> {
                    <<Self as pgorm::ActiveEnum>::Value as pgorm::ActiveEnumValue>::try_get_vec_by(res, index)?
                        .into_iter()
                        .map(|value| <Self as pgorm::ActiveEnum>::try_from_value(&value).map_err(Into::into))
                        .collect()
                }
            }
        );

        quote!(
            #[doc = " Generated by pgorm-macros"]
            #[derive(Debug, Clone, PartialEq, Eq)]
            pub struct #enum_name_iden;

            #[automatically_derived]
            impl pgorm::pgorm_query::SqlName for #enum_name_iden {
                fn unquoted(&self, s: &mut dyn std::fmt::Write) {
                    write!(s, "{}", #enum_name).unwrap();
                }
            }

            #impl_enum_variant_iden

            #[automatically_derived]
            impl pgorm::ActiveEnum for #ident {
                type Value = #rs_type;

                type ValueVec = Vec<#rs_type>;

                fn name() -> pgorm::pgorm_query::Name {
                    pgorm::pgorm_query::Name::new(#enum_name_iden)
                }

                fn to_value(&self) -> Self::Value {
                    match self {
                        #( Self::#variant_idents => #variant_values, )*
                    }
                    .to_owned()
                }

                fn try_from_value(v: &Self::Value) -> std::result::Result<Self, pgorm::Error> {
                    match #val {
                        #( #variant_values => Ok(Self::#variant_idents), )*
                        _ => Err(pgorm::Error::Type(format!(
                            "unexpected value for {} enum: {}",
                            stringify!(#ident),
                            v
                        ))),
                    }
                }

                fn db_type() -> pgorm::ColumnDef {
                    pgorm::prelude::ColumnTypeTrait::def(#db_type)
                }
            }

            #impl_try_getable_array

            #[automatically_derived]
            #[allow(clippy::from_over_into)]
            impl Into<pgorm::pgorm_query::Value> for #ident {
                fn into(self) -> pgorm::pgorm_query::Value {
                    <Self as pgorm::ActiveEnum>::to_value(&self).into()
                }
            }

            #[automatically_derived]
            impl pgorm::TryGetable for #ident {
                fn try_get_by<I: pgorm::RowIndex + std::fmt::Display>(res: &pgorm::QueryResult, idx: I) -> std::result::Result<Self, pgorm::TryGetError> {
                    let value = <<Self as pgorm::ActiveEnum>::Value as pgorm::TryGetable>::try_get_by(res, idx)?;
                    <Self as pgorm::ActiveEnum>::try_from_value(&value).map_err(pgorm::TryGetError::Db)
                }
            }

            #[automatically_derived]
            impl pgorm::pgorm_query::ValueType for #ident {
                fn try_from(v: pgorm::pgorm_query::Value) -> std::result::Result<Self, pgorm::pgorm_query::ValueTypeError> {
                    let value = <<Self as pgorm::ActiveEnum>::Value as pgorm::pgorm_query::ValueType>::try_from(v)?;
                    <Self as pgorm::ActiveEnum>::try_from_value(&value).map_err(|_| pgorm::pgorm_query::ValueTypeError)
                }

                fn type_name() -> String {
                    <<Self as pgorm::ActiveEnum>::Value as pgorm::pgorm_query::ValueType>::type_name()
                }

                fn array_type() -> pgorm::pgorm_query::ArrayType {
                    <<Self as pgorm::ActiveEnum>::Value as pgorm::pgorm_query::ValueType>::array_type()
                }

                fn column_type() -> pgorm::pgorm_query::ColumnType {
                    <Self as pgorm::ActiveEnum>::db_type()
                        .get_column_type()
                        .to_owned()
                        .into()
                }
            }

            #[automatically_derived]
            impl pgorm::pgorm_query::Nullable for #ident {
                fn null() -> pgorm::pgorm_query::Value {
                    <<Self as pgorm::ActiveEnum>::Value as pgorm::pgorm_query::Nullable>::null()
                }
            }

            #impl_not_u8
        )
    }
}

pub fn expand_derive_active_enum(input: syn::DeriveInput) -> syn::Result<TokenStream> {
    let ident_span = input.ident.span();

    match ActiveEnum::new(input) {
        Ok(model) => model.expand(),
        Err(Error::InputNotEnum) => Ok(quote_spanned! {
            ident_span => compile_error!("you can only derive `ActiveEnum` on enums");
        }),
        Err(Error::TT(token_stream)) => Ok(token_stream),
        Err(Error::Syn(e)) => Err(e),
    }
}
