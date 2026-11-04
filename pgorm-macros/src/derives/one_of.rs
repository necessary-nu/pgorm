use super::disjoint::once;
use super::util::{bare_code, trim_starting_raw_identifier, unknown_pgorm_key};
use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::{
    Attribute, Data, DeriveInput, Expr, Fields, Ident, Meta, Token, Type, Variant,
    punctuated::Punctuated, spanned::Spanned,
};

/// The container's keys: the entity holding the reference, and attributes
/// `<Enum>Key` and `<Enum>Model` carry besides their own.
struct Container {
    entity: Type,
    key_attrs: Vec<Meta>,
    model_attrs: Vec<Meta>,
}

/// One variant naming a target: its entity, and the column of the
/// referencing entity holding its key.
struct Target {
    variant: Ident,
    entity: Type,
    column: Expr,
    docs: Vec<Attribute>,
}

/// A variant: a target, or the unit variant, a row referencing none.
enum Reference {
    Target(Box<Target>),
    None {
        variant: Ident,
        docs: Vec<Attribute>,
    },
}

impl Reference {
    fn target(&self) -> Option<&Target> {
        match self {
            Self::Target(target) => Some(target),
            Self::None { .. } => None,
        }
    }

    fn unit(&self) -> Option<&Ident> {
        match self {
            Self::Target(_) => None,
            Self::None { variant, .. } => Some(variant),
        }
    }
}

/// The attributes `key_attrs(..)` or `model_attrs(..)` give, each written as
/// it would be on the type.
fn attrs(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Vec<Meta>> {
    let content;
    syn::parenthesized!(content in meta.input);
    Ok(Punctuated::<Meta, Token![,]>::parse_terminated(&content)?
        .into_iter()
        .collect())
}

// [spec:pgorm:def:disjoint.one-of+2]
// [spec:pgorm:req:macros.derive.code-values+3]
fn container(input: &DeriveInput) -> syn::Result<Container> {
    let mut entity = None;
    let mut key_attrs = None;
    let mut model_attrs = None;
    for attr in input
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("pgorm"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("entity") {
                once(&meta, &mut entity, bare_code::<Type>(&meta, "entity")?)
            } else if meta.path.is_ident("key_attrs") {
                once(&meta, &mut key_attrs, attrs(&meta)?)
            } else if meta.path.is_ident("model_attrs") {
                once(&meta, &mut model_attrs, attrs(&meta)?)
            } else {
                Err(unknown_pgorm_key(&meta))
            }
        })?;
    }
    let Some(entity) = entity else {
        return Err(syn::Error::new(
            input.ident.span(),
            "`OneOf` names the entity whose rows hold the reference: \
             `#[pgorm(entity = comment::Entity)]`",
        ));
    };
    Ok(Container {
        entity,
        key_attrs: key_attrs.unwrap_or_default(),
        model_attrs: model_attrs.unwrap_or_default(),
    })
}

// [spec:pgorm:def:disjoint.one-of+2]
// [spec:pgorm:req:macros.derive.code-values+3]
fn reference(variant: &Variant) -> syn::Result<Reference> {
    let mut column = None;
    for attr in variant
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("pgorm"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("column") {
                once(&meta, &mut column, bare_code::<Expr>(&meta, "column")?)
            } else {
                Err(unknown_pgorm_key(&meta))
            }
        })?;
    }
    if let Some((_, discriminant)) = &variant.discriminant {
        return Err(syn::Error::new_spanned(
            discriminant,
            "a `OneOf` variant names a target entity, or is the unit variant of a row \
             referencing none: it takes no discriminant",
        ));
    }
    let docs = variant
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("doc"))
        .cloned()
        .collect();
    let variant_ident = variant.ident.clone();
    match &variant.fields {
        Fields::Unit => match column {
            Some(column) => Err(syn::Error::new_spanned(
                column,
                "the unit variant is a row referencing none of the targets, so it has no column",
            )),
            None => Ok(Reference::None {
                variant: variant_ident,
                docs,
            }),
        },
        Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
            let field = &fields.unnamed[0];
            if let Some(attr) = field.attrs.iter().find(|a| a.path().is_ident("pgorm")) {
                return Err(syn::Error::new_spanned(
                    attr,
                    "a target entity takes no `#[pgorm(..)]`: its column is the variant's \
                     `column`",
                ));
            }
            let Some(column) = column else {
                return Err(syn::Error::new(
                    variant.ident.span(),
                    "a variant naming a target names the column holding its key: \
                     `#[pgorm(column = Column::PostId)]`",
                ));
            };
            Ok(Reference::Target(Box::new(Target {
                variant: variant_ident,
                entity: field.ty.clone(),
                column,
                docs,
            })))
        }
        fields => Err(syn::Error::new_spanned(
            fields,
            "a `OneOf` variant names its target entity as its one unnamed field \
             (`Post(post::Entity)`), or is a unit variant, a row referencing none",
        )),
    }
}

/// `#[derive(OneOf)]`: `<Enum>Key` and `<Enum>Model`, `OneOf` for the enum
/// and `OneOfTarget` for each target, with the checks that hold each column
/// to the entity, to its target's key type and to one variant.
// [spec:pgorm:def:disjoint.one-of+2]
pub fn expand_derive_one_of(input: DeriveInput) -> syn::Result<TokenStream> {
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new(
            input.ident.span(),
            "you can only derive `OneOf` on enums",
        ));
    };
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "a `OneOf` enum takes no generic parameters",
        ));
    }
    let container = container(&input)?;
    let mut references: Vec<Reference> = Vec::new();
    for variant in &data.variants {
        let reference = reference(variant)?;
        if let Reference::None { variant, .. } = &reference
            && let Some(first) = references.iter().find_map(Reference::unit)
        {
            return Err(syn::Error::new(
                variant.span(),
                format!(
                    "`{first}` is already the unit variant: a row references none of the \
                     targets in one way"
                ),
            ));
        }
        references.push(reference);
    }
    let targets: Vec<&Target> = references.iter().filter_map(Reference::target).collect();
    let none = references.iter().find_map(Reference::unit);
    if targets.is_empty() {
        return Err(syn::Error::new(
            input.ident.span(),
            "a `OneOf` enum references one of several entities, and this enum has no variant \
             naming one: `#[pgorm(column = Column::PostId)] Post(post::Entity)`",
        ));
    }
    let mut expanded = generated_enums(&input, &container, &references);
    expanded.extend(impl_one_of(&input, &container, &targets, none));
    expanded.extend(
        targets
            .iter()
            .map(|target| impl_target(&input.ident, &container, target)),
    );
    expanded.extend(distinct_columns(&targets));
    Ok(expanded)
}

/// `<Enum>Key` and `<Enum>Model`: the enum's variants, each naming a target
/// holding its key and its `Model`, and the unit variant holding nothing.
///
/// Both repeat the enum's variants, which clippy's `enum_variant_names`
/// judges where the enum is declared, so it is allowed on them.
// [spec:pgorm:def:disjoint.one-of+2]
fn generated_enums(
    input: &DeriveInput,
    container: &Container,
    references: &[Reference],
) -> TokenStream {
    let ident = &input.ident;
    let vis = &input.vis;
    let key_ident = format_ident!("{}Key", ident);
    let model_ident = format_ident!("{}Model", ident);
    let variants = |held: &dyn Fn(&Type) -> TokenStream| -> Vec<TokenStream> {
        references
            .iter()
            .map(|reference| match reference {
                Reference::Target(target) => {
                    let Target {
                        variant,
                        entity,
                        docs,
                        ..
                    } = &**target;
                    let held = held(entity);
                    quote_spanned! {entity.span()=> #(#docs)* #variant(#held), }
                }
                Reference::None { variant, docs } => quote!(#(#docs)* #variant,),
            })
            .collect()
    };
    let keys = variants(&|entity| quote!(pgorm::TargetKey<#entity>));
    let models = variants(&|entity| quote!(<#entity as pgorm::EntityTrait>::Model));
    let key_doc = format!(
        " The key a [`{ident}`] reference holds: each target's key, in the variant naming it."
    );
    let model_doc = format!(
        " The target a [`{ident}`] reference loads as: each target's `Model`, in the variant \
         naming it."
    );
    let key_attrs = &container.key_attrs;
    let model_attrs = &container.model_attrs;
    quote! {
        #[doc = #key_doc]
        #[derive(Clone, Debug, PartialEq)]
        #[allow(clippy::enum_variant_names)]
        #( #[#key_attrs] )*
        #vis enum #key_ident {
            #( #keys )*
        }

        #[doc = #model_doc]
        #[derive(Clone, Debug)]
        #[allow(clippy::enum_variant_names)]
        #( #[#model_attrs] )*
        #vis enum #model_ident {
            #( #models )*
        }
    }
}

/// `OneOf` for the enum: its entity, its generated enums, its name in
/// snake_case, its unit variant, and a visit of its targets, each requiring
/// the entity's relation to the target.
// [spec:pgorm:def:disjoint.one-of+2]
fn impl_one_of(
    input: &DeriveInput,
    container: &Container,
    targets: &[&Target],
    none: Option<&Ident>,
) -> TokenStream {
    let ident = &input.ident;
    let entity = &container.entity;
    let key_ident = format_ident!("{}Key", ident);
    let model_ident = format_ident!("{}Model", ident);
    let name = trim_starting_raw_identifier(ident).to_snake_case();
    let none = match none {
        Some(variant) => quote! {
            ::core::option::Option::Some((#key_ident::#variant, #model_ident::#variant))
        },
        None => quote!(::core::option::Option::None),
    };
    let visits = targets.iter().map(|Target { entity, .. }| {
        quote_spanned! {entity.span()=> visitor.target::<#entity>(); }
    });
    quote! {
        #[automatically_derived]
        impl pgorm::OneOf for #ident {
            type Entity = #entity;
            type Key = #key_ident;
            type Model = #model_ident;

            fn name() -> &'static str {
                #name
            }

            fn none() -> ::core::option::Option<(#key_ident, #model_ident)> {
                #none
            }

            fn visit_targets<V: pgorm::TargetVisitor<Self>>(visitor: &mut V) {
                #( #visits )*
            }
        }
    }
}

/// `OneOfTarget` for one target's entity. The column is typed as the
/// entity's `Column`, so another entity's does not compile, and is read
/// through the field its entity's `Model` declares for it, bound as `Option`
/// of the target's key type, so a field of any other type does not compile.
// [spec:pgorm:def:disjoint.one-of+2]
fn impl_target(ident: &Ident, container: &Container, target: &Target) -> TokenStream {
    let base = &container.entity;
    let key_ident = format_ident!("{}Key", ident);
    let model_ident = format_ident!("{}Model", ident);
    let Target {
        variant,
        entity,
        column,
        ..
    } = target;
    let column_const = quote_spanned! {column.span()=>
        const COLUMN: <#base as pgorm::EntityTrait>::Column = #column;
    };
    let read = quote_spanned! {column.span()=>
        let field: &::core::option::Option<pgorm::TargetKey<Self>> =
            <<#base as pgorm::EntityTrait>::Model as pgorm::ColumnField<{ (#column) as usize }>>::field(model);
        ::core::clone::Clone::clone(field)
    };
    quote_spanned! {entity.span()=>
        #[automatically_derived]
        impl pgorm::OneOfTarget<#ident> for #entity {
            #column_const

            #[allow(clippy::clone_on_copy)]
            fn read(
                model: &<#base as pgorm::EntityTrait>::Model,
            ) -> ::core::option::Option<pgorm::TargetKey<Self>> {
                #read
            }

            fn write(
                active: &mut <#base as pgorm::EntityTrait>::ActiveModel,
                key: ::core::option::Option<&pgorm::TargetKey<Self>>,
            ) -> ::core::result::Result<(), pgorm::Error> {
                pgorm::ActiveModelTrait::set(
                    active,
                    <Self as pgorm::OneOfTarget<#ident>>::COLUMN,
                    key.cloned().into(),
                )
            }

            fn of_key(key: &#key_ident) -> ::core::option::Option<&pgorm::TargetKey<Self>> {
                #[allow(unreachable_patterns)]
                match key {
                    #key_ident::#variant(key) => ::core::option::Option::Some(key),
                    _ => ::core::option::Option::None,
                }
            }

            fn key(key: pgorm::TargetKey<Self>) -> #key_ident {
                #key_ident::#variant(key)
            }

            fn model(model: <Self as pgorm::EntityTrait>::Model) -> #model_ident {
                #model_ident::#variant(model)
            }
        }
    }
}

/// A constant for each pair of targets refusing, as it is evaluated, two
/// variants naming one column. Both columns are the entity's `Column`, so
/// equal discriminants are one column, however each path is written.
// [spec:pgorm:def:disjoint.one-of+2]
fn distinct_columns(targets: &[&Target]) -> TokenStream {
    let mut checks = TokenStream::new();
    for (index, second) in targets.iter().enumerate() {
        for first in targets.iter().take(index) {
            let message = format!(
                "`{}` and `{}` name one column: each target's key is held in a column of its own",
                first.variant, second.variant
            );
            let (a, b) = (&first.column, &second.column);
            checks.extend(quote_spanned! {b.span()=>
                const _: () = {
                    if ((#a) as usize) == ((#b) as usize) {
                        ::core::panic!(#message);
                    }
                };
            });
        }
    }
    checks
}
