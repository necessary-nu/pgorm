use heck::ToUpperCamelCase;
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, GenericArgument, PathArguments, Type, spanned::Spanned};

/// `#[derive(Scope)]`: the type is a scope, one value of which is a context
/// of its own.
// [spec:pgorm:def:scope.declare]
pub fn expand_derive_scope(input: DeriveInput) -> syn::Result<TokenStream> {
    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics pgorm::Scope for #ident #ty_generics #where_clause {}
    })
}

/// The scope a context's field holds: `S` for a field of a scope `S`, and
/// `S` for an `Every<S>`, read from how the field's type is written, since
/// the derive sees no further than that.
fn held_scope(ty: &Type) -> &Type {
    if let Type::Path(path) = ty
        && path.qself.is_none()
        && let Some(last) = path.path.segments.last()
        && last.ident == "Every"
        && let PathArguments::AngleBracketed(args) = &last.arguments
        && let [GenericArgument::Type(scope)] = args.args.iter().collect::<Vec<_>>().as_slice()
    {
        return scope;
    }
    ty
}

/// `#[derive(ScopeContext)]` on a struct of scopes and `Every`s of them: the
/// struct is a context holding each, with a `Provides<S>` for each scope `S`
/// a field holds. A struct of generic parameters, a field holding a scope
/// another field holds, and anything but a struct are refused.
// [spec:pgorm:def:scope.context]
pub fn expand_derive_scope_context(input: DeriveInput) -> syn::Result<TokenStream> {
    let ident = &input.ident;
    if let Some(param) = input.generics.params.first() {
        return Err(syn::Error::new(
            param.span(),
            "a scope context names its scopes' types; it has no generic parameters",
        ));
    }
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            ident.span(),
            "a scope context is a struct of scopes, each field a scope or an `Every` of one",
        ));
    };
    let fields: Vec<(TokenStream, &Type)> = match &data.fields {
        Fields::Named(named) => named
            .named
            .iter()
            .map(|field| {
                let name = &field.ident;
                (quote!(#name), &field.ty)
            })
            .collect(),
        Fields::Unnamed(unnamed) => unnamed
            .unnamed
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let index = syn::Index::from(index);
                (quote!(#index), &field.ty)
            })
            .collect(),
        Fields::Unit => Vec::new(),
    };
    let mut held: Vec<(String, &Type)> = Vec::new();
    for (_, ty) in &fields {
        let scope = held_scope(ty);
        let spelled = quote!(#scope).to_string();
        if held.iter().any(|(seen, _)| *seen == spelled) {
            return Err(syn::Error::new(
                ty.span(),
                format!(
                    "this context already holds `{}`; a context holds each scope once, as \
                     one value of it or as every value of it",
                    scope_label(scope)
                ),
            ));
        }
        held.push((spelled, scope));
    }
    let lookups = fields.iter().map(|(name, _)| {
        quote! { .or_else(|| pgorm::ScopeContext::held(&self.#name, scope)) }
    });
    let provides = held.iter().map(|(_, scope)| {
        quote! {
            #[automatically_derived]
            impl pgorm::Provides<#scope> for #ident {}
        }
    });
    Ok(quote! {
        #[automatically_derived]
        impl pgorm::ScopeContext for #ident {
            fn held(
                &self,
                scope: ::std::any::TypeId,
            ) -> ::std::option::Option<pgorm::Held<'_, dyn ::std::any::Any>> {
                ::std::option::Option::None #( #lookups )*
            }
        }

        #( #provides )*
    })
}

/// How a scope is named in a message: the last segment of the path its type
/// is written as, or the type as written.
pub(crate) fn scope_label(ty: &Type) -> String {
    match ty {
        Type::Path(path) if path.qself.is_none() => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_else(|| quote!(#ty).to_string()),
        _ => quote!(#ty).to_string(),
    }
}

/// `` `A` ``, `` `A` and `B` ``, `` `A`, `B` and `C` ``.
fn listed(labels: &[String]) -> String {
    let quoted: Vec<String> = labels.iter().map(|label| format!("`{label}`")).collect();
    match quoted.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// The scopes an entity declares: each column `#[pgorm(scope)]` marks, by its
/// `Column` variant and the field's type, which is the scope; and each type
/// the container's `scope(..)` names, whose `ScopeOn` the program writes.
#[derive(Default)]
pub(crate) struct DeclaredScopes {
    pub(crate) columns: Vec<(Ident, Type)>,
    pub(crate) written: Vec<Type>,
}

/// The name of the trait an entity's scopes are checked through: the table's
/// in UpperCamelCase, or `Model`'s for an entity whose names are computed, so
/// that no two entities of a crate share one by default.
fn checking_trait(table_name: Option<&str>) -> Ident {
    let base: String = table_name
        .map(|table| {
            table
                .to_upper_camel_case()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect()
        })
        .unwrap_or_default();
    match base.chars().next() {
        Some(first) if first.is_ascii_alphabetic() => format_ident!("{base}Scopes"),
        Some(_) => format_ident!("Table{base}Scopes"),
        None => format_ident!("ModelScopes"),
    }
}

/// `EntityScopes` and `ScopedIn` for the entity `Entity`, and for a scoped
/// one the trait of its own its statements' contexts are checked through,
/// whose message names the entity and its scopes.
///
/// An unscoped entity's `ScopedIn` holds under every context, and is marked
/// `do_not_recommend` so that rustc does not offer it where a scoped one's
/// is missing. A scoped one's holds under a context that is its trait's,
/// which every context providing each of its scopes is; that impl is the one
/// marked, so rustc reports the entity's own trait, and its message, rather
/// than the `Provides` a context lacks.
// [spec:pgorm:req:scope.compile]
// [spec:pgorm:def:scope.declare]
pub(crate) fn expand_entity_scopes(
    scopes: &DeclaredScopes,
    table_name: Option<&str>,
) -> TokenStream {
    if scopes.columns.is_empty() && scopes.written.is_empty() {
        return quote! {
            #[automatically_derived]
            impl pgorm::EntityScopes for Entity {}

            #[automatically_derived]
            #[diagnostic::do_not_recommend]
            impl<Cx: pgorm::ScopeContext + ?Sized> pgorm::ScopedIn<Cx> for Entity {}
        };
    }
    let mut types: Vec<&Type> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    for ty in scopes
        .columns
        .iter()
        .map(|(_, ty)| ty)
        .chain(scopes.written.iter())
    {
        let label = scope_label(ty);
        if !labels.contains(&label) {
            labels.push(label);
            types.push(ty);
        }
    }
    let names = labels.clone();
    let list = listed(&labels);
    let message = format!("`{{E}}` is scoped by {list}, which `{{Self}}` does not hold");
    let label = format!("`{{E}}` needs a context holding {list}");
    let note = "a statement on `{E}` starts from a `_for` form, given a context holding its \
                scopes first: `{E}::find_for(&cx)`, `Insert::one_for(&cx, row)`, \
                `Update::one_for(&cx, row)`";
    let checking = checking_trait(table_name);
    let column_conditions = scopes.columns.iter().map(|(variant, ty)| {
        let name = scope_label(ty);
        quote! { .column::<#ty, _>(#name, Column::#variant)? }
    });
    let written_conditions: Vec<TokenStream> = scopes
        .written
        .iter()
        .map(|ty| {
            let name = scope_label(ty);
            quote! { .written::<#ty>(#name)? }
        })
        .collect();
    let column_conditions: Vec<TokenStream> = column_conditions.collect();
    let column_fills = scopes.columns.iter().map(|(variant, ty)| {
        let name = scope_label(ty);
        quote! { .column::<#ty>(#name, Column::#variant)? }
    });
    let written_fills = scopes.written.iter().map(|ty| {
        let name = scope_label(ty);
        quote! { .written::<#ty>(#name)? }
    });
    let doc = format!(
        " The contexts the statements of this module's `Entity` are built under: those \
         holding {list}."
    );
    quote! {
        #[doc = #doc]
        #[diagnostic::on_unimplemented(message = #message, label = #label, note = #note)]
        pub trait #checking<E> {}

        #[automatically_derived]
        #[diagnostic::do_not_recommend]
        impl<Cx: ?Sized #( + pgorm::Provides<#types> )*> #checking<Entity> for Cx {}

        #[automatically_derived]
        impl<Cx: ?Sized + pgorm::ScopeContext + #checking<Entity>> pgorm::ScopedIn<Cx> for Entity {}

        #[automatically_derived]
        impl pgorm::EntityScopes for Entity {
            fn scopes() -> ::std::option::Option<pgorm::Scopes> {
                fn condition(
                    cx: &dyn pgorm::ScopeContext,
                    table: pgorm::pgorm_query::Name,
                ) -> ::std::result::Result<
                    ::std::option::Option<pgorm::Condition>,
                    pgorm::ScopeError,
                > {
                    ::std::result::Result::Ok(
                        pgorm::ScopeConditions::<Entity>::new(cx, table)
                            #( #column_conditions )*
                            #( #written_conditions )*
                            .finish(),
                    )
                }
                fn piped<'brand>(
                    cx: &dyn pgorm::ScopeContext,
                    qualifier: ::std::string::String,
                    binder: &mut pgorm::pipeline::Binder<'brand>,
                ) -> ::std::result::Result<
                    ::std::option::Option<pgorm::pipeline::Expr<'brand>>,
                    pgorm::ScopeError,
                > {
                    ::std::result::Result::Ok(
                        pgorm::pipeline::ScopeFilters::<Entity>::new(cx, qualifier, binder)
                            #( #column_conditions )*
                            #( #written_conditions )*
                            .finish(),
                    )
                }
                ::std::option::Option::Some(pgorm::Scopes::new(
                    &[#( #names ),*],
                    condition,
                    piped,
                ))
            }

            fn fill_scopes(
                cx: &dyn pgorm::ScopeContext,
                row: &mut <Self as pgorm::EntityTrait>::ActiveModel,
            ) -> ::std::result::Result<(), pgorm::ScopeError>
            where
                Self: pgorm::EntityTrait,
            {
                pgorm::ScopeFill::<Self>::new(cx, row)
                    #( #column_fills )*
                    #( #written_fills )*;
                ::std::result::Result::Ok(())
            }
        }
    }
}
