use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};

use super::attributes::{derive_attr, field_attr};

enum Error {
    InputNotEnum,
    Syn(syn::Error),
}

struct DeriveRelation {
    entity_ident: syn::Ident,
    ident: syn::Ident,
    variants: syn::punctuated::Punctuated<syn::Variant, syn::token::Comma>,
}

impl DeriveRelation {
    fn new(input: syn::DeriveInput) -> Result<Self, Error> {
        let variants = match input.data {
            syn::Data::Enum(syn::DataEnum { variants, .. }) => variants,
            _ => return Err(Error::InputNotEnum),
        };

        let pgorm_attr = derive_attr::Pgorm::try_from_attributes(&input.attrs)
            .map_err(Error::Syn)?
            .unwrap_or_default();

        let ident = input.ident;
        let entity_ident = pgorm_attr.entity.unwrap_or_else(|| format_ident!("Entity"));

        Ok(DeriveRelation {
            entity_ident,
            ident,
            variants,
        })
    }

    fn expand(&self) -> syn::Result<TokenStream> {
        let expanded_impl_relation_trait = self.impl_relation_trait()?;

        Ok(expanded_impl_relation_trait)
    }

    fn impl_relation_trait(&self) -> syn::Result<TokenStream> {
        let ident = &self.ident;
        let entity_ident = &self.entity_ident;
        let no_relation_def_msg = format!("No RelationDef for {ident}");

        let variant_relation_defs: Vec<TokenStream> = self
            .variants
            .iter()
            .map(|variant| {
                let variant_ident = &variant.ident;
                let attr = field_attr::Pgorm::from_attributes(&variant.attrs)?;
                let mut relation_type = quote! { error };
                let related_to = if attr.belongs_to.is_some() {
                    relation_type = quote! { belongs_to };
                    attr.belongs_to
                        .as_ref()
                        .map(Self::parse_lit_string)
                        .ok_or_else(|| {
                            syn::Error::new_spanned(variant, "Missing value for 'belongs_to'")
                        })
                } else if attr.has_one.is_some() {
                    relation_type = quote! { has_one };
                    attr.has_one
                        .as_ref()
                        .map(Self::parse_lit_string)
                        .ok_or_else(|| {
                            syn::Error::new_spanned(variant, "Missing value for 'has_one'")
                        })
                } else if attr.has_many.is_some() {
                    relation_type = quote! { has_many };
                    attr.has_many
                        .as_ref()
                        .map(Self::parse_lit_string)
                        .ok_or_else(|| {
                            syn::Error::new_spanned(variant, "Missing value for 'has_many'")
                        })
                } else {
                    Err(syn::Error::new_spanned(
                        variant,
                        "Missing one of 'has_one', 'has_many' or 'belongs_to'",
                    ))
                }??;

                let mut result = quote!(
                    Self::#variant_ident => #entity_ident::#relation_type(#related_to)
                );

                match (attr.from.as_ref(), attr.to.as_ref()) {
                    (Some(from), Some(to)) => {
                        let pairs = Self::column_pairs(variant, from, to)?;
                        let mut pairs = pairs.into_iter();
                        if let Some((from, to)) = pairs.next() {
                            result = quote! { #result.columns(#from, #to) };
                        }
                        for (from, to) in pairs {
                            result = quote! { #result.and_columns(#from, #to) };
                        }
                    }
                    (None, Some(_)) => {
                        return Err(syn::Error::new_spanned(variant, "Missing attribute 'from'"));
                    }
                    (Some(_), None) => {
                        return Err(syn::Error::new_spanned(variant, "Missing attribute 'to'"));
                    }
                    (None, None) if attr.belongs_to.is_some() => {
                        return Err(syn::Error::new_spanned(variant, "Missing attribute 'from'"));
                    }
                    (None, None) => {}
                }
                result.extend(Self::period(variant, &attr)?);

                if attr.on_update.is_some() {
                    let on_update = attr
                        .on_update
                        .as_ref()
                        .map(Self::parse_lit_string)
                        .ok_or_else(|| {
                            syn::Error::new_spanned(variant, "Missing value for 'on_update'")
                        })??;
                    result =
                        quote! { #result.on_update(pgorm::prelude::ForeignKeyAction::#on_update) };
                }

                if attr.on_delete.is_some() {
                    let on_delete = attr
                        .on_delete
                        .as_ref()
                        .map(Self::parse_lit_string)
                        .ok_or_else(|| {
                            syn::Error::new_spanned(variant, "Missing value for 'on_delete'")
                        })??;
                    result =
                        quote! { #result.on_delete(pgorm::prelude::ForeignKeyAction::#on_delete) };
                }

                if let Some(lit) = &attr.on_condition {
                    let on_condition = Self::on_condition(lit)?;
                    result = quote! { #result.on_condition(#on_condition) };
                }

                if attr.fk_name.is_some() {
                    let fk_name = attr
                        .fk_name
                        .as_ref()
                        .map(|lit| match lit {
                            syn::Lit::Str(lit_str) => Ok(lit_str.value()),
                            _ => Err(syn::Error::new_spanned(lit, "attribute must be a string")),
                        })
                        .ok_or_else(|| {
                            syn::Error::new_spanned(variant, "Missing value for 'fk_name'")
                        })??;
                    result = quote! { #result.fk_name(#fk_name) };
                }

                if attr.condition_type.is_some() {
                    let condition_type = attr
                        .condition_type
                        .as_ref()
                        .map(|lit| match lit {
                            syn::Lit::Str(lit_str) => {
                                match lit_str.value().to_ascii_lowercase().as_str() {
                                    "all" => Ok(quote!(pgorm::pgorm_query::ConditionType::All)),
                                    "any" => Ok(quote!(pgorm::pgorm_query::ConditionType::Any)),
                                    _ => Err(syn::Error::new_spanned(
                                        lit,
                                        "Condition type must be one of `all` or `any`",
                                    )),
                                }
                            }
                            _ => Err(syn::Error::new_spanned(lit, "attribute must be a string")),
                        })
                        .ok_or_else(|| {
                            syn::Error::new_spanned(variant, "Missing value for 'condition_type'")
                        })??;
                    result = quote! { #result.condition_type(#condition_type) };
                }

                result.extend(Self::key_check(&attr)?);
                result = quote! { #result.into() };

                Result::<_, syn::Error>::Ok(result)
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(quote!(
            #[automatically_derived]
            impl pgorm::entity::RelationTrait for #ident {
                fn def(&self) -> pgorm::entity::RelationDef {
                    match self {
                        #( #variant_relation_defs, )*
                        _ => panic!(#no_relation_def_msg)
                    }
                }
            }
        ))
    }

    /// The `from_period` / `to_period` pair a temporal relation matches for
    /// overlap, as the builder call that sets it. The two come together and
    /// after a `from` / `to` pair, since PostgreSQL refuses a period on one
    /// side alone (42830) and a key the period is the only column of (42601);
    /// and a temporal foreign key takes no referential action but `NO
    /// ACTION`, PostgreSQL 18 refusing the rest (0A000).
    // [spec:pgorm:syn:macros.derive.relation+4]
    fn period(variant: &syn::Variant, attr: &field_attr::Pgorm) -> syn::Result<TokenStream> {
        let (from, to) = match (&attr.from_period, &attr.to_period) {
            (Some(from), Some(to)) => (from, to),
            (None, None) => return Ok(TokenStream::new()),
            (Some(_), None) => {
                return Err(syn::Error::new_spanned(
                    variant,
                    "Missing attribute 'to_period'",
                ));
            }
            (None, Some(_)) => {
                return Err(syn::Error::new_spanned(
                    variant,
                    "Missing attribute 'from_period'",
                ));
            }
        };
        if attr.from.is_none() {
            return Err(syn::Error::new_spanned(
                from,
                "a period is matched after the columns matched for equality; \
                 give 'from' and 'to' as well",
            ));
        }
        for action in [&attr.on_update, &attr.on_delete].into_iter().flatten() {
            if !matches!(action, syn::Lit::Str(name) if name.value() == "NoAction") {
                return Err(syn::Error::new_spanned(
                    action,
                    "a PERIOD foreign key takes no referential action but NoAction; \
                     PostgreSQL 18 refuses the rest",
                ));
            }
        }
        let from = Self::parse_lit_expr(from)?;
        let to = Self::parse_lit_expr(to)?;
        Ok(quote! { .period(#from, #to) })
    }

    /// The closure an `on_condition` value becomes, taking the two tables as
    /// the join binds them. A closure `|left, right| ..` is that closure, its
    /// parameters typed as the names they are where it left them untyped; any
    /// other expression is the condition whatever the tables are called.
    // [spec:pgorm:req:macros.derive.one-form]
    // [spec:pgorm:syn:macros.derive.relation+4]
    fn on_condition(lit: &syn::Lit) -> syn::Result<TokenStream> {
        let mut closure = match Self::parse_lit_expr(lit)? {
            syn::Expr::Closure(closure) => closure,
            condition => {
                return Ok(quote! {
                    |_, _| pgorm::pgorm_query::IntoCondition::into_condition(#condition)
                });
            }
        };
        if closure.inputs.len() != 2 {
            return Err(syn::Error::new_spanned(
                lit,
                "an `on_condition` closure takes the two tables the join binds, \
                 `|left, right|`",
            ));
        }
        for input in closure.inputs.iter_mut() {
            if !matches!(input, syn::Pat::Type(_)) {
                let pat = input.clone();
                *input = syn::Pat::Type(syn::PatType {
                    attrs: Vec::new(),
                    pat: Box::new(pat),
                    colon_token: Default::default(),
                    ty: Box::new(syn::parse_quote!(pgorm::pgorm_query::Name)),
                });
            }
        }
        Ok(quote! {
            {
                let on_condition = #closure;
                move |left, right| {
                    pgorm::pgorm_query::IntoCondition::into_condition(on_condition(left, right))
                }
            }
        })
    }

    /// The `enforcement` and `deferrability` a relation's foreign key is
    /// declared with, as builder calls: each value names a variant of its enum
    /// (`"NotEnforced"`, `"DeferrableInitiallyDeferred"`), and any other name
    /// is refused here, where its span is still known.
    // [spec:pgorm:syn:macros.derive.relation+4]
    fn key_check(attr: &field_attr::Pgorm) -> syn::Result<TokenStream> {
        let mut calls = TokenStream::new();
        if let Some(lit) = &attr.enforcement {
            let variant = Self::variant_of(lit, "enforcement", &["Enforced", "NotEnforced"])?;
            calls.extend(quote! { .enforcement(pgorm::pgorm_query::Enforcement::#variant) });
        }
        if let Some(lit) = &attr.deferrability {
            let variant = Self::variant_of(
                lit,
                "deferrability",
                &[
                    "NotDeferrable",
                    "DeferrableInitiallyImmediate",
                    "DeferrableInitiallyDeferred",
                ],
            )?;
            calls.extend(quote! { .deferrability(pgorm::pgorm_query::Deferrability::#variant) });
        }
        Ok(calls)
    }

    /// The variant `lit` names, one of `variants`.
    fn variant_of(lit: &syn::Lit, key: &str, variants: &[&str]) -> syn::Result<syn::Ident> {
        let syn::Lit::Str(lit_str) = lit else {
            return Err(syn::Error::new_spanned(lit, "attribute must be a string"));
        };
        let value = lit_str.value();
        if variants.contains(&value.as_str()) {
            Ok(format_ident!("{}", value, span = lit_str.span()))
        } else {
            Err(syn::Error::new_spanned(
                lit,
                format!("'{key}' must be one of {}", variants.join(", ")),
            ))
        }
    }

    /// Pair the `from` and `to` attributes column by column.
    ///
    /// Both sides are parsed as expressions and a tuple on either side is taken
    /// apart, so a composite key becomes one pair per column. Sides of unequal
    /// length are rejected here, where the arity is still known: the builder
    /// takes pairs, and a join that constrains fewer columns than the relation
    /// declares has no way to be expressed.
    fn column_pairs(
        variant: &syn::Variant,
        from: &syn::Lit,
        to: &syn::Lit,
    ) -> syn::Result<Vec<(syn::Expr, syn::Expr)>> {
        let from = Self::parse_lit_expr(from)?;
        let to = Self::parse_lit_expr(to)?;
        let from = Self::split_tuple(from);
        let to = Self::split_tuple(to);

        if from.len() != to.len() {
            return Err(syn::Error::new_spanned(
                variant,
                format!(
                    "'from' names {} column(s) and 'to' names {}; a relation joins its columns in pairs",
                    from.len(),
                    to.len(),
                ),
            ));
        }

        Ok(from.into_iter().zip(to).collect())
    }

    fn split_tuple(expr: syn::Expr) -> Vec<syn::Expr> {
        match expr {
            syn::Expr::Tuple(tuple) => tuple.elems.into_iter().collect(),
            expr => vec![expr],
        }
    }

    fn parse_lit_expr(lit: &syn::Lit) -> syn::Result<syn::Expr> {
        match lit {
            syn::Lit::Str(lit_str) => lit_str
                .parse()
                .map_err(|_| syn::Error::new_spanned(lit, "attribute not valid")),
            _ => Err(syn::Error::new_spanned(lit, "attribute must be a string")),
        }
    }

    fn parse_lit_string(lit: &syn::Lit) -> syn::Result<TokenStream> {
        match lit {
            syn::Lit::Str(lit_str) => lit_str
                .value()
                .parse()
                .map_err(|_| syn::Error::new_spanned(lit, "attribute not valid")),
            _ => Err(syn::Error::new_spanned(lit, "attribute must be a string")),
        }
    }
}

/// Method to derive a Relation
// [spec:pgorm:syn:macros.derive.relation+4]
pub fn expand_derive_relation(input: syn::DeriveInput) -> syn::Result<TokenStream> {
    let ident_span = input.ident.span();

    match DeriveRelation::new(input) {
        Ok(model) => model.expand(),
        Err(Error::InputNotEnum) => Ok(quote_spanned! {
            ident_span => compile_error!("you can only derive DeriveRelation on enums");
        }),
        Err(Error::Syn(err)) => Err(err),
    }
}
