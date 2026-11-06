use heck::ToUpperCamelCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::BTreeMap;

use super::ident::safe_ident;
use crate::{Error, OneOfEnum, TableIdent, util::escape_rust_keyword};

/// The unit variant of an enum whose `CHECK` lets a row reference none.
const NONE: &str = "None";

impl OneOfEnum {
    /// The enum as the derive reads it, after its entity's `Relation` enum.
    ///
    /// Its variants are named by the schema, for the target tables, so
    /// clippy's `enum_variant_names` is allowed on it as on a disjoint enum.
    // [spec:pgorm:def:disjoint.one-of+4]
    pub(crate) fn tokens(&self) -> TokenStream {
        let ident = format_ident!("{}", self.name);
        let variants = self.targets.iter().map(|target| {
            let variant = format_ident!("{}", target.variant);
            let column = format_ident!(
                "{}",
                escape_rust_keyword(target.column.to_upper_camel_case())
            );
            let entity = match &target.module {
                Some(module) => {
                    let module = format_ident!("{}", module);
                    quote! { super::#module::Entity }
                }
                None => quote! { Entity },
            };
            quote! {
                #[pgorm(column = Column::#column)]
                #variant(#entity)
            }
        });
        let none = self.none.then(|| {
            let none = format_ident!("{}", NONE);
            quote! { #none, }
        });
        quote! {
            #[allow(clippy::enum_variant_names)]
            #[derive(Clone, Copy, Debug, OneOf)]
            #[pgorm(entity = Entity)]
            pub enum #ident {
                #( #variants, )*
                #none
            }
        }
    }

    /// The items the enum's derive declares in the module, each with what
    /// it is: the enum, `<Enum>Key` and `<Enum>Model`.
    // [spec:pgorm:def:disjoint.one-of+4]
    pub(crate) fn items(&self) -> [(String, &'static str); 3] {
        [
            (self.name.clone(), "enum"),
            (format!("{}Key", self.name), "key enum"),
            (format!("{}Model", self.name), "model enum"),
        ]
    }

    /// Refuse a variant named as another of the enum's, or an enum named as
    /// no Rust identifier.
    // [spec:pgorm:def:disjoint.one-of+4]
    pub(crate) fn refuse_variant_clashes(&self, table: &TableIdent) -> Result<(), Error> {
        safe_ident(
            &format!(
                "table `{table}`: the one-of reference named for constraint `{}`",
                self.constraint
            ),
            &self.name,
        )?;
        let mut named: BTreeMap<&str, String> = BTreeMap::new();
        if self.none {
            named.insert(NONE, "its unit variant".to_owned());
        }
        for target in &self.targets {
            let other = format!("the variant for column `{}`", target.column);
            if let Some(first) = named.insert(&target.variant, other) {
                return Err(Error::TransformError(format!(
                    "table `{table}`: its one-of reference's variant `{}` is named as {first}",
                    target.variant
                )));
            }
        }
        Ok(())
    }
}
