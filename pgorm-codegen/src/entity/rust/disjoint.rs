use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use crate::{DisjointEnum, Entity, KindColumn, util::escape_rust_keyword};

/// The active-model enum the derive declares for the enum named `ident`.
pub(crate) fn active_model_ident(ident: &str) -> String {
    format!("{ident}ActiveModel")
}

/// The name the derive gives the kind type of the enum named `ident` when no
/// `enum_name` says otherwise.
fn default_kind_type_name(ident: &str) -> String {
    let bare = ident.strip_prefix("r#").unwrap_or(ident);
    format!("{}_kind", bare.to_snake_case())
}

impl DisjointEnum {
    /// The enum as the derive reads it, saying only what differs from its
    /// defaults: `kind` where the kind column's variant is not `Kind`,
    /// `enum_name` where the kind type is not named for the enum, and
    /// `schema_name` where the type is qualified; and a variant's `value`
    /// where it is not the variant's name in snake_case. An unkinded enum
    /// says `unkinded` and none of them, and takes no `kind_attrs`, since no
    /// `Model` holds its kind enum.
    ///
    /// Its variants are named by the schema, for the kinds' values and
    /// tables, not by the program, so clippy's `enum_variant_names`, which a
    /// variant ending with the enum's name (`Shop::WebShop`) trips, is
    /// allowed on it.
    // [spec:pgorm:sem:codegen.entity.disjoint+3]
    // [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1]
    pub(crate) fn tokens(
        &self,
        base: &Entity,
        extra: &TokenStream,
        kind_derives: Option<TokenStream>,
    ) -> TokenStream {
        let ident = base.get_table_name_camel_case_ident().to_string();
        let enum_iden = format_ident!("{}", ident);
        let eq = self.eq.then(|| quote! { , Eq });
        let mut keys = vec![quote! { base = Entity }];
        match &self.kinded {
            Some(kinded) => keys.extend(kinded.keys(&ident, kind_derives)),
            None => keys.push(quote! { unkinded }),
        }
        let variants = self.kinds.iter().map(|kind| {
            let variant = format_ident!("{}", kind.variant);
            let attr = kind
                .value
                .as_ref()
                .map(|value| quote! { #[pgorm(value = #value)] });
            match &kind.table {
                Some(module) => {
                    let module = format_ident!("{}", module);
                    quote! { #attr #variant(super::#module::Model) }
                }
                None => quote! { #attr #variant },
            }
        });
        quote! {
            #[allow(clippy::enum_variant_names)]
            #[derive(Clone, Debug, PartialEq #eq, Disjoint #extra)]
            #[pgorm(#(#keys),*)]
            pub enum #enum_iden {
                #( #variants, )*
            }
        }
    }
}

impl KindColumn {
    /// The keys naming the kind column and the kind type of the enum named
    /// `ident` where they differ from the derive's defaults, and the kind
    /// type's derives.
    // [spec:pgorm:sem:codegen.entity.disjoint+3]
    fn keys(&self, ident: &str, kind_derives: Option<TokenStream>) -> Vec<TokenStream> {
        let mut keys = Vec::new();
        let column: Ident =
            format_ident!("{}", escape_rust_keyword(self.column.to_upper_camel_case()));
        if column != "Kind" {
            keys.push(quote! { kind = Column::#column });
        }
        let kind_type = self.kind_type.to_string();
        if kind_type != default_kind_type_name(ident) {
            keys.push(quote! { enum_name = #kind_type });
        }
        if let Some(schema) = &self.kind_schema {
            let schema = schema.to_string();
            keys.push(quote! { schema_name = #schema });
        }
        if let Some(derives) = kind_derives {
            keys.push(quote! { kind_attrs(derive(#derives)) });
        }
        keys
    }
}
