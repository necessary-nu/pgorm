use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::{Expr, LitStr, Path, meta::ParseNestedMeta};

use super::util::{COMPOSITE_KEYS, bare_code, unknown_pgorm_key};

enum Error {
    InputNotEnum,
    Syn(syn::Error),
}

struct Relation {
    entity_ident: syn::Ident,
    ident: syn::Ident,
    variants: syn::punctuated::Punctuated<syn::Variant, syn::token::Comma>,
}

/// Which of an entity's relations a variant is, the builder method of the
/// same name.
#[derive(Clone, Copy)]
enum Kind {
    BelongsTo,
    HasOne,
    HasMany,
}

impl Kind {
    fn of(meta: &ParseNestedMeta<'_>) -> Option<(Self, &'static str)> {
        [
            (Self::BelongsTo, "belongs_to"),
            (Self::HasOne, "has_one"),
            (Self::HasMany, "has_many"),
        ]
        .into_iter()
        .find(|(_, key)| meta.path.is_ident(key))
    }

    fn builder(self) -> TokenStream {
        match self {
            Self::BelongsTo => quote! { belongs_to },
            Self::HasOne => quote! { has_one },
            Self::HasMany => quote! { has_many },
        }
    }
}

/// One variant's `#[pgorm(...)]` keys, accumulated across every attribute on
/// it. Each value that is Rust code is written bare and taken as written; a
/// key given twice is refused at the second.
#[derive(Default)]
struct VariantAttrs {
    /// The relation's kind and the entity it reaches.
    target: Option<(Kind, Path)>,
    from: Option<Expr>,
    to: Option<Expr>,
    from_period: Option<Expr>,
    to_period: Option<Expr>,
    on_update: Option<Expr>,
    on_delete: Option<Expr>,
    on_condition: Option<Expr>,
    fk_name: Option<LitStr>,
    condition_type: Option<Expr>,
    enforcement: Option<Expr>,
    deferrability: Option<Expr>,
}

impl VariantAttrs {
    // [spec:pgorm:syn:macros.derive.relation+6]
    // [spec:pgorm:req:macros.derive.code-values+2]
    fn parse(variant: &syn::Variant) -> syn::Result<Self> {
        let mut attrs = Self::default();
        for attr in variant
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("pgorm"))
        {
            attr.parse_nested_meta(|meta| attrs.read(&meta))?;
        }
        Ok(attrs)
    }

    fn read(&mut self, meta: &ParseNestedMeta<'_>) -> syn::Result<()> {
        if let Some((kind, key)) = Kind::of(meta) {
            if self.target.is_some() {
                return Err(
                    meta.error("a relation is one of `belongs_to`, `has_one` or `has_many`, once")
                );
            }
            self.target = Some((kind, bare_code(meta, key)?));
            return Ok(());
        }
        let key = meta
            .path
            .get_ident()
            .map(ToString::to_string)
            .unwrap_or_default();
        if key == "fk_name" {
            let name = meta.value()?.parse()?;
            return once(meta, &mut self.fk_name, name);
        }
        let slot = match key.as_str() {
            "from" => &mut self.from,
            "to" => &mut self.to,
            "from_period" => &mut self.from_period,
            "to_period" => &mut self.to_period,
            "on_update" => &mut self.on_update,
            "on_delete" => &mut self.on_delete,
            "on_condition" => &mut self.on_condition,
            "condition_type" => &mut self.condition_type,
            "enforcement" => &mut self.enforcement,
            "deferrability" => &mut self.deferrability,
            _ => return Err(unknown_pgorm_key(meta)),
        };
        let value = bare_code(meta, &key)?;
        once(meta, slot, value)
    }
}

/// Fill `slot` with `value`, refusing a key given twice at the second.
fn once<T>(meta: &ParseNestedMeta<'_>, slot: &mut Option<T>, value: T) -> syn::Result<()> {
    if slot.is_some() {
        let key = meta
            .path
            .get_ident()
            .map(ToString::to_string)
            .unwrap_or_default();
        return Err(meta.error(format!("duplicate `{key}`")));
    }
    *slot = Some(value);
    Ok(())
}

/// The entity the relations belong to, from the container's `entity`, the one
/// key the derive reads there.
// [spec:pgorm:syn:macros.derive.relation+6]
fn relation_entity(attrs: &[syn::Attribute]) -> syn::Result<Option<syn::Ident>> {
    let mut entity = None;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("pgorm")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("entity") {
                let value = bare_code(&meta, "entity")?;
                return once(&meta, &mut entity, value);
            }
            match meta.path.get_ident() {
                Some(key) if COMPOSITE_KEYS.contains(&key.to_string().as_str()) => {
                    Err(meta.error(format!(
                        "`{key}` is the composite derive's: it goes on the entity's `Model` \
                         under `Entity`; `Relation` reads `entity`"
                    )))
                }
                _ => Err(unknown_pgorm_key(&meta)),
            }
        })?;
    }
    Ok(entity)
}

impl Relation {
    fn new(input: syn::DeriveInput) -> Result<Self, Error> {
        let variants = match input.data {
            syn::Data::Enum(syn::DataEnum { variants, .. }) => variants,
            _ => return Err(Error::InputNotEnum),
        };

        let entity_ident = relation_entity(&input.attrs)
            .map_err(Error::Syn)?
            .unwrap_or_else(|| format_ident!("Entity"));

        Ok(Relation {
            entity_ident,
            ident: input.ident,
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
                let attr = VariantAttrs::parse(variant)?;
                let Some((kind, target)) = &attr.target else {
                    return Err(syn::Error::new_spanned(
                        variant,
                        "Missing one of 'has_one', 'has_many' or 'belongs_to'",
                    ));
                };
                let builder = kind.builder();

                let mut result = quote!(
                    Self::#variant_ident => #entity_ident::#builder(#target)
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
                    (None, None) if matches!(kind, Kind::BelongsTo) => {
                        return Err(syn::Error::new_spanned(variant, "Missing attribute 'from'"));
                    }
                    (None, None) => {}
                }
                result.extend(Self::period(variant, &attr)?);

                if let Some(on_update) = &attr.on_update {
                    result = quote! { #result.on_update(#on_update) };
                }
                if let Some(on_delete) = &attr.on_delete {
                    result = quote! { #result.on_delete(#on_delete) };
                }
                if let Some(on_condition) = &attr.on_condition {
                    let on_condition = Self::on_condition(on_condition)?;
                    result = quote! { #result.on_condition(#on_condition) };
                }
                if let Some(fk_name) = &attr.fk_name {
                    result = quote! { #result.fk_name(#fk_name) };
                }
                if let Some(condition_type) = &attr.condition_type {
                    result = quote! { #result.condition_type(#condition_type) };
                }
                if let Some(enforcement) = &attr.enforcement {
                    result = quote! { #result.enforcement(#enforcement) };
                }
                if let Some(deferrability) = &attr.deferrability {
                    result = quote! { #result.deferrability(#deferrability) };
                }
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
    /// ACTION`, PostgreSQL 18 refusing the rest (0A000), so an action is a
    /// path ending in `NoAction` or refused.
    // [spec:pgorm:syn:macros.derive.relation+6]
    fn period(variant: &syn::Variant, attr: &VariantAttrs) -> syn::Result<TokenStream> {
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
            let no_action = matches!(
                action,
                Expr::Path(path) if path.path.segments.last().is_some_and(|last| last.ident == "NoAction")
            );
            if !no_action {
                return Err(syn::Error::new_spanned(
                    action,
                    "a PERIOD foreign key takes no referential action but \
                     `ForeignKeyAction::NoAction`; PostgreSQL 18 refuses the rest",
                ));
            }
        }
        Ok(quote! { .period(#from, #to) })
    }

    /// The closure an `on_condition` value becomes, taking the two tables as
    /// the join binds them. A closure `|left, right| ..` is that closure, its
    /// parameters typed as the names they are where it left them untyped; any
    /// other expression is the condition whatever the tables are called.
    // [spec:pgorm:req:macros.derive.one-form]
    // [spec:pgorm:syn:macros.derive.relation+6]
    fn on_condition(condition: &Expr) -> syn::Result<TokenStream> {
        let mut closure = match condition.clone() {
            Expr::Closure(closure) => closure,
            condition => {
                return Ok(quote! {
                    |_, _| pgorm::pgorm_query::IntoCondition::into_condition(#condition)
                });
            }
        };
        if closure.inputs.len() != 2 {
            return Err(syn::Error::new_spanned(
                condition,
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

    /// Pair the `from` and `to` attributes column by column.
    ///
    /// Both sides are parsed as expressions and a tuple on either side is taken
    /// apart, so a composite key becomes one pair per column. Sides of unequal
    /// length are rejected here, where the arity is still known: the builder
    /// takes pairs, and a join that constrains fewer columns than the relation
    /// declares has no way to be expressed.
    fn column_pairs(
        variant: &syn::Variant,
        from: &Expr,
        to: &Expr,
    ) -> syn::Result<Vec<(Expr, Expr)>> {
        let from = Self::split_tuple(from.clone());
        let to = Self::split_tuple(to.clone());

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
}

/// Method to derive a Relation
// [spec:pgorm:syn:macros.derive.relation+6]
pub fn expand_derive_relation(input: syn::DeriveInput) -> syn::Result<TokenStream> {
    let ident_span = input.ident.span();

    match Relation::new(input) {
        Ok(model) => model.expand(),
        Err(Error::InputNotEnum) => Ok(quote_spanned! {
            ident_span => compile_error!("you can only derive `Relation` on enums");
        }),
        Err(Error::Syn(err)) => Err(err),
    }
}
