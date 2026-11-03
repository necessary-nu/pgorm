use super::active_enum::expand_derive_active_enum;
use super::util::{bare_code, trim_starting_raw_identifier, unknown_pgorm_key};
use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::{
    Attribute, Data, DeriveInput, Expr, Fields, Ident, LitStr, Meta, Token, Type, Variant,
    meta::ParseNestedMeta, parse_quote, punctuated::Punctuated, spanned::Spanned,
};

/// The container's keys: the base entity, the column holding each row's kind,
/// the kind type's SQL name, and attributes the kind type carries besides its
/// own.
struct Container {
    base: Type,
    kind: Option<Expr>,
    enum_name: Option<LitStr>,
    schema_name: Option<LitStr>,
    kind_attrs: Vec<Meta>,
}

/// One variant: a kind of base row.
struct Kind {
    variant: Ident,
    /// The kind type's value for the kind, as `value` gave it or else the
    /// variant's name in snake_case.
    value: String,
    /// The `value` key, which the kind type's expansion is given as the
    /// variant's `string_value`.
    given: Option<LitStr>,
    /// The `Model` of the kind's entity, for a kind with a table.
    payload: Option<Type>,
    docs: Vec<Attribute>,
}

/// Fill `slot` from `meta`, refusing a key given twice at the second.
fn once<T>(meta: &ParseNestedMeta<'_>, slot: &mut Option<T>, value: T) -> syn::Result<()> {
    if slot.is_some() {
        let key = meta
            .path
            .get_ident()
            .map(Ident::to_string)
            .unwrap_or_default();
        return Err(meta.error(format!("`{key}` is given twice")));
    }
    *slot = Some(value);
    Ok(())
}

// [spec:pgorm:def:disjoint.declare+3]
// [spec:pgorm:req:macros.derive.code-values+2]
fn container(input: &DeriveInput) -> syn::Result<Container> {
    let mut base = None;
    let mut kind = None;
    let mut enum_name = None;
    let mut schema_name = None;
    let mut kind_attrs = None;
    for attr in input
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("pgorm"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("base") {
                once(&meta, &mut base, bare_code::<Type>(&meta, "base")?)
            } else if meta.path.is_ident("kind") {
                once(&meta, &mut kind, bare_code::<Expr>(&meta, "kind")?)
            } else if meta.path.is_ident("enum_name") {
                once(&meta, &mut enum_name, meta.value()?.parse::<LitStr>()?)
            } else if meta.path.is_ident("schema_name") {
                once(&meta, &mut schema_name, meta.value()?.parse::<LitStr>()?)
            } else if meta.path.is_ident("kind_attrs") {
                let content;
                syn::parenthesized!(content in meta.input);
                let attrs = Punctuated::<Meta, Token![,]>::parse_terminated(&content)?;
                once(&meta, &mut kind_attrs, attrs.into_iter().collect())
            } else {
                Err(unknown_pgorm_key(&meta))
            }
        })?;
    }
    let Some(base) = base else {
        return Err(syn::Error::new(
            input.ident.span(),
            "`Disjoint` names the entity its kinds are rows of: \
             `#[pgorm(base = vehicle::Entity)]`",
        ));
    };
    Ok(Container {
        base,
        kind,
        enum_name,
        schema_name,
        kind_attrs: kind_attrs.unwrap_or_default(),
    })
}

// [spec:pgorm:def:disjoint.declare+3]
fn kind(variant: &Variant) -> syn::Result<Kind> {
    let mut given = None;
    for attr in variant
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("pgorm"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("value") {
                once(&meta, &mut given, meta.value()?.parse::<LitStr>()?)
            } else {
                Err(unknown_pgorm_key(&meta))
            }
        })?;
    }
    if let Some((_, discriminant)) = &variant.discriminant {
        return Err(syn::Error::new_spanned(
            discriminant,
            "a kind's value is its variant's name in snake_case or its `value`, \
             not a discriminant",
        ));
    }
    let payload = match &variant.fields {
        Fields::Unit => None,
        Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
            let field = &fields.unnamed[0];
            if let Some(attr) = field.attrs.iter().find(|a| a.path().is_ident("pgorm")) {
                return Err(syn::Error::new_spanned(
                    attr,
                    "a kind's `Model` takes no `#[pgorm(..)]`: its entity is configured \
                     where it is declared",
                ));
            }
            Some(field.ty.clone())
        }
        fields => {
            return Err(syn::Error::new_spanned(
                fields,
                "a kind is a unit variant, with no table of its own (`Bike`), or holds its \
                 entity's `Model` as its one unnamed field (`Car(car::Model)`)",
            ));
        }
    };
    Ok(Kind {
        variant: variant.ident.clone(),
        value: given.as_ref().map_or_else(
            || trim_starting_raw_identifier(&variant.ident).to_snake_case(),
            LitStr::value,
        ),
        given,
        payload,
        docs: variant
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("doc"))
            .cloned()
            .collect(),
    })
}

/// `#[derive(Disjoint)]`: the kind type, the enum of the kinds' active models,
/// and the traits a disjoint entity's schema, reads and writes are built on.
// [spec:pgorm:def:disjoint.declare+3]
pub fn expand_derive_disjoint(input: DeriveInput) -> syn::Result<TokenStream> {
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new(
            input.ident.span(),
            "you can only derive `Disjoint` on enums",
        ));
    };
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "a disjoint entity's enum takes no generic parameters",
        ));
    }
    let container = container(&input)?;
    if data.variants.is_empty() {
        return Err(syn::Error::new(
            input.ident.span(),
            "a disjoint entity has a variant for each kind of base row, and this enum has none",
        ));
    }

    let mut kinds: Vec<Kind> = Vec::new();
    for variant in &data.variants {
        let kind = kind(variant)?;
        if let Some(first) = kinds.iter().find(|first| first.value == kind.value) {
            let span = kind
                .given
                .as_ref()
                .map_or_else(|| kind.variant.span(), LitStr::span);
            return Err(syn::Error::new(
                span,
                format!(
                    "`{}` is already the value of `{}`: each kind is one value of the kind type",
                    kind.value, first.variant
                ),
            ));
        }
        kinds.push(kind);
    }

    let ident = &input.ident;
    let enum_name = container.enum_name.as_ref().map_or_else(
        || {
            format!(
                "{}_kind",
                trim_starting_raw_identifier(ident).to_snake_case()
            )
        },
        LitStr::value,
    );
    let mut expanded = kind_type(&input, &container, &kinds, &enum_name)?;
    expanded.extend(active_model(&input, &kinds));
    expanded.extend(impl_disjoint(&input, &container, &kinds));
    Ok(expanded)
}

/// `<Enum>Kind`, and its `ActiveEnum` from the expansion the `ActiveEnum`
/// derive writes, given the type's name and each value the derive was told;
/// `kind_attrs` follow in the order given, so a helper attribute can follow
/// the derive in them that introduces it.
// [spec:pgorm:def:disjoint.declare+3]
fn kind_type(
    input: &DeriveInput,
    container: &Container,
    kinds: &[Kind],
    enum_name: &str,
) -> syn::Result<TokenStream> {
    let ident = &input.ident;
    let vis = &input.vis;
    let kind_ident = format_ident!("{}Kind", ident);
    let doc = format!(
        " The kinds of a [`{ident}`]: the PostgreSQL enum `{enum_name}` its base row's kind \
         column holds."
    );
    let variants: Vec<&Ident> = kinds.iter().map(|kind| &kind.variant).collect();
    let docs = kinds.iter().map(|kind| {
        let doc = format!(
            " A [`{ident}::{}`], stored as `{}`.",
            kind.variant, kind.value
        );
        quote!(#[doc = #doc])
    });
    let schema_name = container
        .schema_name
        .as_ref()
        .map(|schema| quote!(, schema_name = #schema));
    let string_values = kinds.iter().map(|kind| {
        kind.given
            .as_ref()
            .map(|value| quote!(#[pgorm(string_value = #value)]))
    });
    let active_enum: DeriveInput = parse_quote! {
        #[pgorm(enum_name = #enum_name #schema_name)]
        #vis enum #kind_ident {
            #( #string_values #variants, )*
        }
    };
    let active_enum = expand_derive_active_enum(active_enum)?;
    let kind_attrs = &container.kind_attrs;
    Ok(quote! {
        #[doc = #doc]
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
        #( #[#kind_attrs] )*
        #vis enum #kind_ident {
            #( #docs #variants, )*
        }

        #active_enum
    })
}

/// `<Enum>ActiveModel`: the enum's variants, each holding its kind entity's
/// `ActiveModel` in place of its `Model`.
// [spec:pgorm:def:disjoint.declare+3]
fn active_model(input: &DeriveInput, kinds: &[Kind]) -> TokenStream {
    let ident = &input.ident;
    let vis = &input.vis;
    let active_ident = format_ident!("{}ActiveModel", ident);
    let doc = format!(
        " A [`{ident}`] to write: each kind with a table holds its entity's `ActiveModel`."
    );
    let variants = kinds.iter().map(|kind| {
        let docs = &kind.docs;
        let variant = &kind.variant;
        match &kind.payload {
            Some(payload) => quote_spanned! {payload.span()=>
                #(#docs)*
                #variant(<<#payload as pgorm::ModelTrait>::Entity as pgorm::EntityTrait>::ActiveModel)
            },
            None => quote!(#(#docs)* #variant),
        }
    });
    quote! {
        #[doc = #doc]
        #[derive(Clone, Debug, PartialEq)]
        #vis enum #active_ident {
            #( #variants, )*
        }
    }
}

/// `Disjoint` for the enum, `KindModel` for each kind's `Model` and
/// `DisjointActiveModel` for `<Enum>ActiveModel`.
///
/// Each `Model` is a kind of one enum only once: a second variant holding it
/// is a second `KindModel` impl for the same type, which coherence refuses.
// [spec:pgorm:def:disjoint.declare+3]
fn impl_disjoint(input: &DeriveInput, container: &Container, kinds: &[Kind]) -> TokenStream {
    let ident = &input.ident;
    let kind_ident = format_ident!("{}Kind", ident);
    let active_ident = format_ident!("{}ActiveModel", ident);
    let base = &container.base;
    let kind_column = match &container.kind {
        Some(kind) => quote!(#kind),
        None => quote_spanned! {base.span()=> <#base as pgorm::EntityTrait>::Column::Kind },
    };

    let variants: Vec<&Ident> = kinds.iter().map(|kind| &kind.variant).collect();
    let patterns: Vec<TokenStream> = kinds
        .iter()
        .map(|kind| match kind.payload {
            Some(_) => quote!((_)),
            None => quote!(),
        })
        .collect();
    let visits = kinds.iter().map(|kind| {
        let variant = &kind.variant;
        match &kind.payload {
            Some(payload) => quote_spanned! {payload.span()=> visitor.table::<#payload>(kind) },
            None => quote!(visitor.unit(Self::#variant)),
        }
    });
    let active_visits = kinds.iter().map(|kind| {
        let variant = &kind.variant;
        match &kind.payload {
            Some(payload) => quote_spanned! {payload.span()=>
                Self::#variant(active) => visitor.table::<#payload>(#kind_ident::#variant, active)
            },
            None => quote!(Self::#variant => visitor.unit(#kind_ident::#variant)),
        }
    });
    let kind_models = kinds.iter().filter_map(|kind| {
        let variant = &kind.variant;
        let payload = kind.payload.as_ref()?;
        Some(quote_spanned! {payload.span()=>
            #[automatically_derived]
            impl pgorm::KindModel<#ident> for #payload {
                const KIND: #kind_ident = #kind_ident::#variant;

                fn into_variant(self) -> #ident {
                    #ident::#variant(self)
                }
            }
        })
    });

    quote! {
        #[automatically_derived]
        impl pgorm::Disjoint for #ident {
            type Base = #base;
            type Kind = #kind_ident;
            type ActiveModel = #active_ident;

            fn kind_column() -> <#base as pgorm::EntityTrait>::Column {
                #kind_column
            }

            fn kind(&self) -> #kind_ident {
                match self {
                    #( Self::#variants #patterns => #kind_ident::#variants, )*
                }
            }

            fn visit_kind<V: pgorm::KindVisitor<Self>>(kind: #kind_ident, visitor: V) -> V::Output {
                match kind {
                    #( #kind_ident::#variants => #visits, )*
                }
            }
        }

        #( #kind_models )*

        #[automatically_derived]
        impl pgorm::DisjointActiveModel for #active_ident {
            type Disjoint = #ident;

            fn kind(&self) -> #kind_ident {
                match self {
                    #( Self::#variants #patterns => #kind_ident::#variants, )*
                }
            }

            fn visit<V: pgorm::ActiveKindVisitor<#ident>>(self, visitor: V) -> V::Output {
                match self {
                    #( #active_visits, )*
                }
            }
        }
    }
}
