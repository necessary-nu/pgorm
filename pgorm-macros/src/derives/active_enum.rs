use super::case_style::{CaseStyle, CaseStyleHelpers};
use super::util::{bare_code, camel_case_with_escaped_non_uax31};
use heck::ToUpperCamelCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::{Expr, Lit, LitInt, LitStr, Type, UnOp, parse};

enum Error {
    InputNotEnum,
    Syn(syn::Error),
    TT(TokenStream),
}

struct ActiveEnum {
    ident: syn::Ident,
    enum_name: String,
    rs_type: Type,
    /// The column type the values are stored as, the expression `db_type`
    /// wrote, or else the PostgreSQL enum of the string values.
    db_type: TokenStream,
    is_string: bool,
    variants: Vec<ActiveEnumVariant>,
    rename_all: Option<CaseStyle>,
    display: bool,
}

struct ActiveEnumVariant {
    ident: syn::Ident,
    string_value: Option<LitStr>,
    num_value: Option<LitInt>,
    rename: Option<CaseStyle>,
    display_value: Option<LitStr>,
}

impl ActiveEnum {
    // [spec:pgorm:syn:macros.derive.active-enum+4]
    // [spec:pgorm:req:macros.derive.code-values+2]
    fn new(input: syn::DeriveInput) -> Result<Self, Error> {
        let ident_span = input.ident.span();
        let ident = input.ident;

        let mut enum_name = ident.to_string().to_upper_camel_case();
        let mut rs_type = Err(Error::TT(quote_spanned! {
            ident_span => compile_error!("Missing macro attribute `rs_type`");
        }));
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
                        rs_type = Ok(bare_code(&meta, "rs_type")?);
                    } else if meta.path.is_ident("db_type") {
                        db_type = Some(bare_code(&meta, "db_type")?);
                    } else if meta.path.is_ident("schema_name") {
                        let litstr: LitStr = meta.value()?.parse()?;
                        schema_name = Some(litstr.value());
                    } else if meta.path.is_ident("enum_name") {
                        let litstr: LitStr = meta.value()?.parse()?;
                        enum_name = litstr.value();
                    } else if meta.path.is_ident("rename_all") {
                        rename_all = Some((&meta).try_into()?);
                    } else if meta.path.is_ident("display") {
                        // [spec:pgorm:req:macros.derive.names]
                        display = true;
                    } else {
                        return Err(meta.error(format!(
                            "Unknown attribute parameter found: {:?}",
                            meta.path.get_ident()
                        )));
                    }
                    Ok(())
                })
                .map_err(Error::Syn)
            })?;

        let variant_vec = match input.data {
            syn::Data::Enum(syn::DataEnum { variants, .. }) => variants,
            _ => return Err(Error::InputNotEnum),
        };

        let mut is_string = rename_all.is_some();
        let mut is_int = false;
        let mut variants = Vec::new();

        for variant in variant_vec {
            let variant_span = variant.ident.span();
            let mut string_value = None;
            let mut num_value = None;
            let mut rename_rule = None;
            let mut display_value = None;

            for attr in variant.attrs.iter() {
                if !attr.path().is_ident("pgorm") {
                    continue;
                }
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("string_value") {
                        is_string = true;
                        string_value = Some(meta.value()?.parse::<LitStr>()?);
                    } else if meta.path.is_ident("num_value") {
                        is_int = true;
                        num_value = Some(meta.value()?.parse::<LitInt>()?);
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
                        display_value = Some(value);
                    } else if meta.path.is_ident("rename") {
                        is_string = true;
                        rename_rule = Some((&meta).try_into()?);
                    } else {
                        return Err(meta.error(format!(
                            "Unknown attribute parameter found: {:?}",
                            meta.path.get_ident()
                        )));
                    }

                    Ok(())
                })
                .map_err(Error::Syn)?;
            }

            if is_string && is_int {
                return Err(Error::TT(quote_spanned! {
                    ident_span => compile_error!("All enum variants should specify the same `*_value` macro attribute, either `string_value` or `num_value` but not both");
                }));
            }

            if string_value.is_none() && num_value.is_none() && rename_rule.or(rename_all).is_none()
            {
                match variant.discriminant {
                    Some((_, Expr::Lit(exprlit))) => {
                        if let Lit::Int(litint) = exprlit.lit {
                            is_int = true;
                            num_value = Some(litint);
                        } else {
                            return Err(Error::TT(quote_spanned! {
                                variant_span => compile_error!("Enum variant discriminant is not an integer");
                            }));
                        }
                    }
                    //rust doesn't provide negative variants in enums as a single LitInt, this workarounds that
                    Some((_, Expr::Unary(exprnlit))) => {
                        if let UnOp::Neg(_) = exprnlit.op {
                            if let Expr::Lit(exprlit) = *exprnlit.expr
                                && let Lit::Int(litint) = exprlit.lit
                            {
                                let negative_token = quote! { -#litint };
                                let litint = parse(negative_token.into()).unwrap();

                                is_int = true;
                                num_value = Some(litint);
                            }
                        } else {
                            return Err(Error::TT(quote_spanned! {
                                variant_span => compile_error!("Only - token is supported in enum variants, not ! and *");
                            }));
                        }
                    }
                    _ => {
                        return Err(Error::TT(quote_spanned! {
                            variant_span => compile_error!("Missing macro attribute, either `string_value`, `num_value` or `rename` should be specified or specify repr[X] and have a value for every entry");
                        }));
                    }
                }
            }

            variants.push(ActiveEnumVariant {
                ident: variant.ident,
                string_value,
                num_value,
                rename: rename_rule,
                display_value,
            });
        }

        // With no `db_type`, the values are a PostgreSQL enum's, which only
        // an enum of strings has.
        let db_type = match db_type {
            Some(db_type) => quote! { #db_type },
            None if is_string => {
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
            None => {
                return Err(Error::TT(quote_spanned! {
                    ident_span => compile_error!("Missing macro attribute `db_type`: an enum stored as numbers names its column type, `db_type = ColumnType::Integer`");
                }));
            }
        };

        Ok(ActiveEnum {
            ident,
            enum_name,
            rs_type: rs_type?,
            db_type,
            is_string,
            variants,
            rename_all,
            display,
        })
    }

    // [spec:pgorm:sem:macros.derive.active-enum.expansion+4]
    fn expand(&self) -> syn::Result<TokenStream> {
        let mut expanded = self.impl_active_enum();
        if self.display {
            expanded.extend(self.impl_display());
        }

        Ok(expanded)
    }

    // [spec:pgorm:sem:macros.derive.active-enum.expansion+4]
    fn impl_display(&self) -> TokenStream {
        let ident = &self.ident;
        let variant_idents = self.variants.iter().map(|variant| &variant.ident);
        let variant_display = self
            .variants
            .iter()
            .map(|variant| match &variant.display_value {
                Some(value) => value.value(),
                None => variant.ident.to_string(),
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
            rename_all,
            ..
        } = self;

        let variant_idents: Vec<syn::Ident> = variants
            .iter()
            .map(|variant| variant.ident.clone())
            .collect();

        let variant_values: Vec<TokenStream> = variants
            .iter()
            .map(|variant| {
                let variant_span = variant.ident.span();

                if let Some(string_value) = &variant.string_value {
                    let string = string_value.value();
                    quote! { #string }
                } else if let Some(num_value) = &variant.num_value {
                    quote! { #num_value }
                } else if let Some(rename_rule) = variant.rename.or(*rename_all) {
                    let variant_ident = variant.ident.convert_case(Some(rename_rule));
                    quote! { #variant_ident }
                } else {
                    quote_spanned! {
                        variant_span => compile_error!("Missing macro attribute, either `string_value`, `num_value` or `rename_all` should be specified");
                    }
                }
            })
            .collect();

        let val = if *is_string {
            quote! { v.as_ref() }
        } else {
            quote! { v }
        };

        let enum_name_iden = format_ident!("{}Enum", ident);

        let str_variants: Vec<String> = variants
            .iter()
            .filter_map(|variant| {
                variant
                    .string_value
                    .as_ref()
                    .map(|string_value| string_value.value())
                    .or(variant
                        .rename
                        .or(*rename_all)
                        .map(|rename| variant.ident.convert_case(Some(rename))))
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
                #[derive(Debug, Clone, PartialEq, Eq, pgorm::EnumIter)]
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
                        <#enum_variant_iden as pgorm::strum::IntoEnumIterator>::iter()
                            .map(pgorm::pgorm_query::Name::new)
                            .collect()
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
