use heck::ToUpperCamelCase;
use pgorm_query::{ColumnType, Name};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::{Entity, EntityWriter, OutputFile, WithSerde};

/// A range type a schema created, or the multirange created beside it, as
/// the newtype the generated entities name it by: one per type, as an enum
/// type is one `ActiveEnum`.
#[derive(Clone, Debug)]
pub struct CreatedRangeType {
    pub(crate) name: Name,
    pub(crate) schema: Option<Name>,
    pub(crate) subtype: ColumnType,
    /// Whether this is the multirange, held as a `Multirange<T>`, rather than
    /// the range, held as a `Range<T>`.
    pub(crate) multirange: bool,
}

/// The Rust type a range type created over `subtype` ranges over, as the
/// prelude names it — `None` for a subtype pgorm's `RangeSubtype` does not
/// cover, which no newtype can hold.
// [spec:pgorm:sem:codegen.entity.types+13]
pub(crate) fn created_range_subtype(subtype: &ColumnType) -> Option<&'static str> {
    Some(match subtype {
        ColumnType::SmallInteger => "i16",
        ColumnType::Integer => "i32",
        ColumnType::BigInteger => "i64",
        ColumnType::Float => "f32",
        ColumnType::Double => "f64",
        ColumnType::Decimal(_) => "Decimal",
        ColumnType::Char(_) | ColumnType::String(_) | ColumnType::Text => "String",
        ColumnType::Date => "Date",
        ColumnType::Time => "Time",
        ColumnType::Timestamp => "DateTime",
        ColumnType::TimestampWithTimeZone => "DateTimeWithTimeZone",
        ColumnType::Uuid => "Uuid",
        _ => return None,
    })
}

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
    // [spec:pgorm:sem:codegen.entity.serde.derives+3]
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
    // [spec:pgorm:req:codegen.entity.files+2]
    // [spec:pgorm:sem:codegen.entity.imports+4]
    pub fn write_pgorm_range_types(&self, with_serde: &WithSerde) -> OutputFile {
        let mut lines = Vec::new();
        Self::write_doc_comment(&mut lines);
        Self::write(&mut lines, vec![Self::gen_import(with_serde)]);
        lines.push("".to_owned());
        let code_blocks = self
            .ranges
            .values()
            .map(|range| range.impl_created_range(with_serde))
            .collect();
        Self::write(&mut lines, code_blocks);
        OutputFile {
            name: "pgorm_range_types.rs".to_owned(),
            content: lines.join("\n"),
        }
    }

    /// The `use` of each range type's newtype an entity's columns name.
    // [spec:pgorm:sem:codegen.entity.imports+4]
    pub fn gen_import_range_types(entity: &Entity) -> TokenStream {
        let mut imported: Vec<String> = Vec::new();
        let mut imports = TokenStream::new();
        for column in entity.columns.iter() {
            if let ColumnType::CreatedRange { name, .. }
            | ColumnType::CreatedMultirange { name, .. } = column.get_inner_col_type()
                && !imported.contains(&name.to_string())
            {
                imported.push(name.to_string());
                let ident = CreatedRangeType::rs_type(name);
                imports.extend(quote! { use super::pgorm_range_types::#ident; });
            }
        }
        imports
    }
}
