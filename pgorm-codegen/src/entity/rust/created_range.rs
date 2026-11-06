use heck::ToUpperCamelCase;
use pgorm_query::{ColumnType, Name};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::format;
use crate::{
    CreatedRangeType, EntityWriter, OutputFile, WithSerde,
    entity::created_range::created_range_subtype,
};

impl CreatedRangeType {
    /// The newtype's name: the range type's, in UpperCamelCase, as an enum
    /// type's Rust enum is named.
    pub(crate) fn rs_type(name: &Name) -> proc_macro2::Ident {
        format_ident!("{}", name.to_string().to_upper_camel_case())
    }

    /// The newtype, deriving `CreatedRange` under the type's name and
    /// schema. `Eq` is derived unless the subtype is a float, as for a Model,
    /// and the serde derives follow the Model's: a Model serializing a field
    /// of this type needs the type to serialize.
    // [spec:pgorm:sem:codegen.entity.types+13]
    // [spec:pgorm:sem:codegen.entity.serde.derives+4]
    pub fn impl_created_range(&self, with_serde: &WithSerde) -> TokenStream {
        let ident = Self::rs_type(&self.name);
        let Some(subtype) = created_range_subtype(&self.subtype) else {
            unreachable!(
                "range type subtype {:?} reached the writer; \
                 every Column is type-checked when it is built",
                self.subtype
            );
        };
        let subtype: TokenStream = subtype
            .parse()
            .expect("mapped Rust type names are token text");
        let type_name = self.name.to_string();
        let schema_attr = self.schema.as_ref().map(|schema| {
            let schema = schema.to_string();
            quote! { , schema_name = #schema }
        });
        let eq = match self.subtype {
            ColumnType::Float | ColumnType::Double => quote! {},
            _ => quote! { , Eq },
        };
        let (key, held) = match self.multirange {
            false => (quote! { range_name }, quote! { Range }),
            true => (quote! { multirange_name }, quote! { Multirange }),
        };
        let serde_derive = with_serde.extra_derive();
        quote! {
            #[derive(Clone, Debug, PartialEq #eq, CreatedRange #serde_derive)]
            #[pgorm(#key = #type_name #schema_attr)]
            pub struct #ident(pub #held<#subtype>);
        }
    }
}

impl EntityWriter {
    /// `pgorm_range_types.rs`: the newtype of every range type a column
    /// names.
    // [spec:pgorm:req:codegen.entity.files+3]
    // [spec:pgorm:sem:codegen.entity.imports+5]
    pub fn write_pgorm_range_types(&self, with_serde: &WithSerde) -> OutputFile {
        let ranges = self
            .ranges
            .values()
            .map(|range| range.impl_created_range(with_serde));
        OutputFile {
            name: "pgorm_range_types.rs".to_owned(),
            content: format::file(std::iter::once(Self::gen_import(with_serde)).chain(ranges)),
        }
    }
}
