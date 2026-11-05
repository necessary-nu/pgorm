use crate::{
    ActiveEnum, CreatedRangeType, Entity, Error, UnkindedDisjoint, util::escape_rust_keyword,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::{collections::BTreeMap, str::FromStr};
use syn::{punctuated::Punctuated, token::Comma};
use tracing::info;

// [spec:pgorm:def:codegen.entity+7]
#[derive(Clone, Debug)]
pub struct EntityWriter {
    pub(crate) entities: Vec<Entity>,
    pub(crate) enums: BTreeMap<String, ActiveEnum>,
    pub(crate) ranges: BTreeMap<String, CreatedRangeType>,
}

pub struct WriterOutput {
    pub files: Vec<OutputFile>,
}

pub struct OutputFile {
    pub name: String,
    pub content: String,
}

// [spec:pgorm:def:codegen.entity.serde+1]
#[derive(PartialEq, Eq, Debug, Default)]
pub enum WithSerde {
    #[default]
    None,
    Serialize,
    Deserialize,
    Both,
}

/// Every generation option, named. `Default` is the no-flags shape: no
/// serde, `mod.rs`, no unkinded disjoint entity.
// [spec:pgorm:sem:codegen.entity.context+4]
#[derive(Debug, Default)]
pub struct EntityWriterOptions {
    pub with_serde: WithSerde,
    pub with_copy_enums: bool,
    pub schema_name: Option<String>,
    pub lib: bool,
    pub serde_skip_deserializing_primary_key: bool,
    pub serde_skip_hidden_column: bool,
    pub model_extra_derives: Vec<String>,
    pub model_extra_attributes: Vec<String>,
    pub enum_extra_derives: Vec<String>,
    pub enum_extra_attributes: Vec<String>,
    /// The table sets to generate as unkinded disjoint entities, which
    /// [`EntityTransformer::transform`](crate::EntityTransformer::transform)
    /// reads and refuses where a set does not have the shape.
    // [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1]
    pub unkinded_disjoint: Vec<UnkindedDisjoint>,
}

#[derive(Debug)]
pub struct EntityWriterContext {
    pub(crate) with_serde: WithSerde,
    pub(crate) with_copy_enums: bool,
    pub(crate) schema_name: Option<String>,
    pub(crate) lib: bool,
    pub(crate) serde_skip_hidden_column: bool,
    pub(crate) serde_skip_deserializing_primary_key: bool,
    pub(crate) model_extra_derives: TokenStream,
    pub(crate) model_extra_attributes: TokenStream,
    pub(crate) enum_extra_derives: TokenStream,
    pub(crate) enum_extra_attributes: TokenStream,
    pub(crate) unkinded_disjoint: Vec<UnkindedDisjoint>,
}

impl WithSerde {
    // [spec:pgorm:sem:codegen.entity.serde.derives+3]
    pub fn extra_derive(&self) -> TokenStream {
        let mut extra_derive = match self {
            Self::None => {
                quote! {}
            }
            Self::Serialize => {
                quote! {
                    Serialize
                }
            }
            Self::Deserialize => {
                quote! {
                    Deserialize
                }
            }
            Self::Both => {
                quote! {
                    Serialize, Deserialize
                }
            }
        };
        if !extra_derive.is_empty() {
            extra_derive = quote! { , #extra_derive }
        }
        extra_derive
    }
}

/// Converts *_extra_derives argument to token stream. `option` names the
/// option the strings came from, so a rejection points at the caller's field.
// [spec:pgorm:sem:codegen.entity.context+4]    extra-derive normalization
pub(crate) fn bonus_derive<T, I>(option: &str, extra_derives: I) -> Result<TokenStream, Error>
where
    T: Into<String>,
    I: IntoIterator<Item = T>,
{
    extra_derives
        .into_iter()
        .map(Into::<String>::into)
        .try_fold(TokenStream::default(), |acc, derive| {
            let tokens: TokenStream = parse_bonus(option, &derive)?;
            Ok(quote! { #acc, #tokens })
        })
}

/// convert *_extra_attributes argument to token stream
// [spec:pgorm:sem:codegen.entity.context+4]    extra-attribute normalization
pub(crate) fn bonus_attributes<T, I>(option: &str, attributes: I) -> Result<TokenStream, Error>
where
    T: Into<String>,
    I: IntoIterator<Item = T>,
{
    attributes.into_iter().map(Into::<String>::into).try_fold(
        TokenStream::default(),
        |acc, attribute| {
            let tokens: TokenStream = parse_bonus(option, &attribute)?;
            Ok(quote! {
                #acc
                #[#tokens]
            })
        },
    )
}

// [spec:pgorm:sem:codegen.entity.context+4]
fn parse_bonus(option: &str, source: &str) -> Result<TokenStream, Error> {
    source.parse().map_err(|_| {
        Error::TransformError(format!(
            "`{option}` entry `{source}` is not valid Rust token text"
        ))
    })
}

impl FromStr for WithSerde {
    type Err = crate::Error;

    // [spec:pgorm:def:codegen.entity.serde+1]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "none" => Self::None,
            "serialize" => Self::Serialize,
            "deserialize" => Self::Deserialize,
            "both" => Self::Both,
            v => {
                return Err(crate::Error::TransformError(format!(
                    "Unsupported enum variant '{v}'"
                )));
            }
        })
    }
}

impl EntityWriterContext {
    // [spec:pgorm:sem:codegen.entity.context+4]
    pub fn new(options: EntityWriterOptions) -> Result<Self, Error> {
        Ok(Self {
            with_serde: options.with_serde,
            with_copy_enums: options.with_copy_enums,
            schema_name: options.schema_name,
            lib: options.lib,
            serde_skip_deserializing_primary_key: options.serde_skip_deserializing_primary_key,
            serde_skip_hidden_column: options.serde_skip_hidden_column,
            model_extra_derives: bonus_derive("model_extra_derives", options.model_extra_derives)?,
            model_extra_attributes: bonus_attributes(
                "model_extra_attributes",
                options.model_extra_attributes,
            )?,
            enum_extra_derives: bonus_derive("enum_extra_derives", options.enum_extra_derives)?,
            enum_extra_attributes: bonus_attributes(
                "enum_extra_attributes",
                options.enum_extra_attributes,
            )?,
            unkinded_disjoint: options.unkinded_disjoint,
        })
    }
}

impl EntityWriter {
    // [spec:pgorm:req:codegen.entity.files+2]
    pub fn generate(self, context: &EntityWriterContext) -> WriterOutput {
        let mut files = Vec::new();
        files.extend(self.write_entities(context));
        files.push(self.write_index_file(context.lib));
        files.push(self.write_prelude());
        if !self.enums.is_empty() {
            files.push(self.write_pgorm_active_enums(
                &context.with_serde,
                context.with_copy_enums,
                &context.enum_extra_derives,
                &context.enum_extra_attributes,
            ));
        }
        if !self.ranges.is_empty() {
            files.push(self.write_pgorm_range_types(&context.with_serde));
        }
        WriterOutput { files }
    }

    // [spec:pgorm:sem:codegen.entity.serde.skip]
    pub fn write_entities(&self, context: &EntityWriterContext) -> Vec<OutputFile> {
        self.entities
            .iter()
            .map(|entity| {
                let entity_file = format!("{}.rs", entity.get_table_name_snake_case());
                let column_info = entity
                    .columns
                    .iter()
                    .map(|column| column.get_info())
                    .collect::<Vec<String>>();
                // Serde must be enabled to use this
                let serde_skip_deserializing_primary_key = context
                    .serde_skip_deserializing_primary_key
                    && matches!(context.with_serde, WithSerde::Both | WithSerde::Deserialize);
                let serde_skip_hidden_column = context.serde_skip_hidden_column
                    && matches!(
                        context.with_serde,
                        WithSerde::Both | WithSerde::Serialize | WithSerde::Deserialize
                    );

                info!("Generating {}", entity_file);
                for info in column_info.iter() {
                    info!("    > {}", info);
                }

                let mut lines = Vec::new();
                Self::write_doc_comment(&mut lines);
                let code_blocks = Self::gen_compact_code_blocks(
                    entity,
                    &context.with_serde,
                    &context.schema_name,
                    serde_skip_deserializing_primary_key,
                    serde_skip_hidden_column,
                    &context.model_extra_derives,
                    &context.model_extra_attributes,
                );
                Self::write(&mut lines, code_blocks);
                OutputFile {
                    name: entity_file,
                    content: lines.join("\n\n"),
                }
            })
            .collect()
    }

    // [spec:pgorm:req:codegen.entity.files+2]
    pub fn write_index_file(&self, lib: bool) -> OutputFile {
        let mut lines = Vec::new();
        Self::write_doc_comment(&mut lines);
        let code_blocks: Vec<TokenStream> = self.entities.iter().map(Self::gen_mod).collect();
        Self::write(
            &mut lines,
            vec![quote! {
                pub mod prelude;
            }],
        );
        lines.push("".to_owned());
        Self::write(&mut lines, code_blocks);
        if !self.enums.is_empty() {
            Self::write(
                &mut lines,
                vec![quote! {
                    pub mod pgorm_active_enums;
                }],
            );
        }
        if !self.ranges.is_empty() {
            Self::write(
                &mut lines,
                vec![quote! {
                    pub mod pgorm_range_types;
                }],
            );
        }

        let file_name = match lib {
            true => "lib.rs".to_owned(),
            false => "mod.rs".to_owned(),
        };

        OutputFile {
            name: file_name,
            content: lines.join("\n"),
        }
    }

    pub fn write_prelude(&self) -> OutputFile {
        let mut lines = Vec::new();
        Self::write_doc_comment(&mut lines);
        let code_blocks = self.entities.iter().map(Self::gen_prelude_use).collect();
        Self::write(&mut lines, code_blocks);
        OutputFile {
            name: "prelude.rs".to_owned(),
            content: lines.join("\n"),
        }
    }

    // [spec:pgorm:sem:codegen.entity.enums+8]
    pub fn write_pgorm_active_enums(
        &self,
        with_serde: &WithSerde,
        with_copy_enums: bool,
        extra_derives: &TokenStream,
        extra_attributes: &TokenStream,
    ) -> OutputFile {
        let mut lines = Vec::new();
        Self::write_doc_comment(&mut lines);
        Self::write(&mut lines, vec![Self::gen_import(with_serde)]);
        lines.push("".to_owned());
        let code_blocks = self
            .enums
            .values()
            .map(|active_enum| {
                active_enum.impl_active_enum(
                    with_serde,
                    with_copy_enums,
                    extra_derives,
                    extra_attributes,
                )
            })
            .collect();
        Self::write(&mut lines, code_blocks);
        OutputFile {
            name: "pgorm_active_enums.rs".to_owned(),
            content: lines.join("\n"),
        }
    }

    pub fn write(lines: &mut Vec<String>, code_blocks: Vec<TokenStream>) {
        lines.extend(
            code_blocks
                .into_iter()
                .map(|code_block| code_block.to_string())
                .collect::<Vec<_>>(),
        );
    }

    pub fn write_doc_comment(lines: &mut Vec<String>) {
        let ver = env!("CARGO_PKG_VERSION");
        let comments = vec![format!(
            "//! `pgorm` Entity, @generated by pgorm-codegen {ver}"
        )];
        lines.extend(comments);
        lines.push("".to_owned());
    }

    // [spec:pgorm:req:macros.derive.one-form]
    // [spec:pgorm:def:codegen.entity.compact+7]
    // [spec:pgorm:def:disjoint.one-of+3]
    #[allow(clippy::too_many_arguments)]
    pub fn gen_compact_code_blocks(
        entity: &Entity,
        with_serde: &WithSerde,
        schema_name: &Option<String>,
        serde_skip_deserializing_primary_key: bool,
        serde_skip_hidden_column: bool,
        model_extra_derives: &TokenStream,
        model_extra_attributes: &TokenStream,
    ) -> Vec<TokenStream> {
        let mut imports = Self::gen_import(with_serde);
        imports.extend(Self::gen_import_active_enum(entity));
        imports.extend(Self::gen_import_range_types(entity));
        let mut code_blocks = vec![
            imports,
            Self::gen_compact_model_struct(
                entity,
                with_serde,
                schema_name,
                serde_skip_deserializing_primary_key,
                serde_skip_hidden_column,
                model_extra_derives,
                model_extra_attributes,
            ),
            Self::gen_compact_relation_enum(entity),
        ];
        code_blocks.extend(Self::gen_disjoint_enum(entity, with_serde));
        code_blocks.extend(entity.one_of.iter().map(crate::OneOfEnum::tokens));
        code_blocks.extend(Self::gen_impl_conjunct_related(entity));
        code_blocks
    }

    /// The `#[derive(Disjoint)]` enum of a disjoint entity's base, deriving
    /// `Eq` where every kind's `Model` does, and the serde derives with the
    /// `Model`s that it holds and its kind type, which its base's `Model`
    /// holds, so the base serializes as the kind entities do.
    // [spec:pgorm:sem:codegen.entity.disjoint+3]
    pub fn gen_disjoint_enum(entity: &Entity, with_serde: &WithSerde) -> Option<TokenStream> {
        let disjoint = entity.disjoint.as_ref()?;
        let serde = match with_serde {
            WithSerde::None => None,
            WithSerde::Serialize => Some(quote! { Serialize }),
            WithSerde::Deserialize => Some(quote! { Deserialize }),
            WithSerde::Both => Some(quote! { Serialize, Deserialize }),
        };
        Some(disjoint.tokens(entity, &with_serde.extra_derive(), serde))
    }

    // [spec:pgorm:sem:codegen.entity.imports+4]
    pub fn gen_import(with_serde: &WithSerde) -> TokenStream {
        let prelude_import = quote!(
            use pgorm::entity::prelude::*;
        );

        match with_serde {
            WithSerde::None => prelude_import,
            WithSerde::Serialize => {
                quote! {
                    #prelude_import
                    use serde::Serialize;
                }
            }
            WithSerde::Deserialize => {
                quote! {
                    #prelude_import
                    use serde::Deserialize;
                }
            }
            WithSerde::Both => {
                quote! {
                    #prelude_import
                    use serde::{Deserialize,Serialize};
                }
            }
        }
    }

    // [spec:pgorm:sem:codegen.entity.imports+4]    active-enum imports
    // [spec:pgorm:sem:codegen.entity.disjoint+3]    a kind type from its base's module
    pub fn gen_import_active_enum(entity: &Entity) -> TokenStream {
        let mut imported: Vec<String> = Vec::new();
        let mut imports = TokenStream::new();
        for (name, module) in entity.columns.iter().filter_map(|col| col.enum_import()) {
            if imported.contains(&name) {
                continue;
            }
            let enum_name = format_ident!("{}", name);
            imported.push(name);
            if let Some(module) = module {
                let module = format_ident!("{}", module);
                imports.extend(quote! {
                    use super::#module::#enum_name;
                });
            }
        }
        imports
    }

    // [spec:pgorm:sem:codegen.entity.relations.via+2]
    pub fn gen_impl_conjunct_related(entity: &Entity) -> Vec<TokenStream> {
        let table_name_camel_case = entity.get_table_name_camel_case_ident();
        let via_snake_case = entity.get_conjunct_relations_via_snake_case();
        let to_snake_case = entity.get_conjunct_relations_to_snake_case();
        let to_upper_camel_case = entity.get_conjunct_relations_to_upper_camel_case();
        via_snake_case
            .into_iter()
            .zip(to_snake_case)
            .zip(to_upper_camel_case)
            .map(|((via_snake_case, to_snake_case), to_upper_camel_case)| {
                quote! {
                    impl Related<super::#to_snake_case::Entity> for Entity {
                        fn to() -> RelationDef {
                            super::#via_snake_case::Relation::#to_upper_camel_case.def()
                        }

                        fn via() -> Option<RelationDef> {
                            Some(super::#via_snake_case::Relation::#table_name_camel_case.def().rev())
                        }
                    }
                }
            })
            .collect()
    }

    pub fn gen_mod(entity: &Entity) -> TokenStream {
        let table_name_snake_case_ident = format_ident!(
            "{}",
            escape_rust_keyword(entity.get_table_name_snake_case_ident())
        );
        quote! {
            pub mod #table_name_snake_case_ident;
        }
    }

    pub fn gen_prelude_use(entity: &Entity) -> TokenStream {
        let table_name_snake_case_ident = entity.get_table_name_snake_case_ident();
        let table_name_camel_case_ident = entity.get_table_name_camel_case_ident();
        quote! {
            pub use super::#table_name_snake_case_ident::Entity as #table_name_camel_case_ident;
        }
    }

    // [spec:pgorm:sem:codegen.entity.compact.attrs+13]
    // [spec:pgorm:sem:codegen.entity.compact.model+2]
    // [spec:pgorm:sem:codegen.entity.pk+5]
    #[allow(clippy::too_many_arguments)]
    pub fn gen_compact_model_struct(
        entity: &Entity,
        with_serde: &WithSerde,
        schema_name: &Option<String>,
        serde_skip_deserializing_primary_key: bool,
        serde_skip_hidden_column: bool,
        model_extra_derives: &TokenStream,
        model_extra_attributes: &TokenStream,
    ) -> TokenStream {
        let table_name = entity.table_name.as_str();
        let column_names_snake_case = entity.get_column_names_snake_case();
        let column_rs_types = entity.get_column_rs_types();
        let if_eq_needed = entity.get_eq_needed();
        let primary_keys: Vec<String> = entity
            .primary_keys
            .iter()
            .map(|pk| pk.name.clone())
            .collect();
        let attrs: Vec<TokenStream> = entity
            .columns
            .iter()
            .map(|col| {
                let mut attrs: Punctuated<_, Comma> = Punctuated::new();
                let is_primary_key = primary_keys.contains(&col.name);
                if !col.is_snake_case_name() {
                    let column_name = &col.name;
                    attrs.push(quote! { column_name = #column_name });
                }
                // A key column's attributes are read from that column
                // alone: the derive answers whether the whole key is
                // generated from them, so a serial or identity column
                // elsewhere in the table says nothing of the key.
                if is_primary_key {
                    attrs.push(quote! { primary_key });
                    if entity.without_overlaps && primary_keys.last() == Some(&col.name) {
                        attrs.push(quote! { without_overlaps });
                    } else if !col.auto_increment
                        && col.identity.is_none()
                        && col.generated.is_none()
                    {
                        attrs.push(quote! { auto_increment = false });
                    }
                }
                if let Some(identity) = col.get_identity_attr() {
                    attrs.push(identity);
                }
                if let Some(expression) = col.get_expression_attr() {
                    attrs.push(expression);
                }
                if let Some(ts) = col.get_col_type_attrs() {
                    attrs.extend([ts]);
                    if !col.not_null {
                        attrs.push(quote! { nullable });
                    }
                };
                if col.unique {
                    attrs.push(quote! { unique });
                }
                let mut ts = quote! {};
                if !attrs.is_empty() {
                    for (i, attr) in attrs.into_iter().enumerate() {
                        if i > 0 {
                            ts = quote! { #ts, };
                        }
                        ts = quote! { #ts #attr };
                    }
                    ts = quote! { #[pgorm(#ts)] };
                }
                let serde_attribute = col.get_serde_attribute(
                    is_primary_key,
                    serde_skip_deserializing_primary_key,
                    serde_skip_hidden_column,
                );
                ts = quote! {
                    #ts
                    #serde_attribute
                };
                ts
            })
            .collect();
        let schema_name = match Self::gen_schema_name(entity, schema_name) {
            Some(schema_name) => quote! {
                schema_name = #schema_name,
            },
            None => quote! {},
        };
        let unique_keys = Self::gen_unique_keys(entity);
        let extra_derive = with_serde.extra_derive();

        quote! {
            #[derive(Clone, Debug, PartialEq, Entity #if_eq_needed #extra_derive #model_extra_derives)]
            #[pgorm(
                #schema_name
                table_name = #table_name
                #(, #unique_keys)*
            )]
            #model_extra_attributes
            pub struct Model {
                #(
                    #attrs
                    pub #column_names_snake_case: #column_rs_types,
                )*
            }
        }
    }

    /// The container's `unique(..)` keys, one per unique key over two columns
    /// or more, in the table's order: the constraint's name when it has one,
    /// `nulls_not_distinct` when it says so, and `columns`, a tuple of the
    /// `Column` variants their fields derive.
    // [spec:pgorm:sem:codegen.entity.compact.attrs+13]
    fn gen_unique_keys(entity: &Entity) -> Vec<TokenStream> {
        entity
            .unique_keys
            .iter()
            .map(|key| {
                let name = key.name.as_ref().map(|name| quote! { name = #name, });
                let nulls_not_distinct = key
                    .nulls_not_distinct
                    .then(|| quote! { nulls_not_distinct, });
                let columns = key.columns.iter().map(|name| {
                    let variant = entity
                        .columns
                        .iter()
                        .find(|column| &column.name == name)
                        .map(crate::Column::get_name_camel_case)
                        .expect("a unique key's columns are the table's");
                    quote! { Column::#variant }
                });
                quote! { unique(#name #nulls_not_distinct columns = (#(#columns),*)) }
            })
            .collect()
    }

    /// The entity's `Relation` enum. Its variants are named by the schema,
    /// for the tables it relates to, so clippy's `enum_variant_names`, which
    /// variants sharing an affix (`WebShop`, `PopUpShop`, `ClosedShop`) or one
    /// ending with the enum's name trip, is allowed on it.
    // [spec:pgorm:def:codegen.entity.compact+7]
    pub fn gen_compact_relation_enum(entity: &Entity) -> TokenStream {
        let relation_enum_name = entity.get_relation_enum_name();
        let attrs = entity.get_relation_attrs();
        quote! {
            #[allow(clippy::enum_variant_names)]
            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {
                #(
                    #attrs
                    #relation_enum_name,
                )*
            }
        }
    }

    /// The schema literal an entity's generated attribute carries: the source
    /// table's own qualifier, and only failing that the configured default. An
    /// option cannot move a table into a schema the DDL says it is not in.
    // [spec:pgorm:sem:codegen.entity.context+4]
    pub fn gen_schema_name(entity: &Entity, schema_name: &Option<String>) -> Option<TokenStream> {
        entity
            .effective_schema(schema_name)
            .map(|schema_name| quote! { #schema_name })
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Column, ConjunctRelation, Entity, EntityWriter, PrimaryKey, Relation, RelationType,
        WithSerde,
        entity::writer::{bonus_attributes, bonus_derive},
    };
    use pgorm_query::{ColumnType, ForeignKeyAction, Name, StringLen};
    use pretty_assertions::assert_eq;
    use proc_macro2::TokenStream;
    use quote::quote;
    use std::io::{self, BufRead, BufReader, Read};
    use std::sync::Arc;

    fn setup() -> Vec<Entity> {
        vec![
            Entity {
                table_name: "cake".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: true,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "name".to_owned(),
                        col_type: ColumnType::Text,
                        auto_increment: false,
                        identity: None,
                        not_null: false,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![Relation {
                    ref_table: "fruit".to_owned(),
                    ref_schema: None,
                    columns: vec![],
                    ref_columns: vec![],
                    rel_type: RelationType::HasMany,
                    on_delete: None,
                    on_update: None,
                    enforcement: None,
                    deferrability: None,
                    period: None,
                    self_referencing: false,
                    num_suffix: 0,
                    impl_related: true,
                }],
                conjunct_relations: vec![ConjunctRelation {
                    via: "cake_filling".to_owned(),
                    to: "filling".to_owned(),
                }],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![PrimaryKey {
                    name: "id".to_owned(),
                }],
            },
            Entity {
                table_name: "_cake_filling_".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "cake_id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "filling_id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![
                    Relation {
                        ref_table: "cake".to_owned(),
                        ref_schema: None,
                        columns: vec!["cake_id".to_owned()],
                        ref_columns: vec!["id".to_owned()],
                        rel_type: RelationType::BelongsTo,
                        on_delete: Some(ForeignKeyAction::Cascade),
                        on_update: Some(ForeignKeyAction::Cascade),
                        enforcement: None,
                        deferrability: None,
                        period: None,
                        self_referencing: false,
                        num_suffix: 0,
                        impl_related: true,
                    },
                    Relation {
                        ref_table: "filling".to_owned(),
                        ref_schema: None,
                        columns: vec!["filling_id".to_owned()],
                        ref_columns: vec!["id".to_owned()],
                        rel_type: RelationType::BelongsTo,
                        on_delete: Some(ForeignKeyAction::Cascade),
                        on_update: Some(ForeignKeyAction::Cascade),
                        enforcement: None,
                        deferrability: None,
                        period: None,
                        self_referencing: false,
                        num_suffix: 0,
                        impl_related: true,
                    },
                ],
                conjunct_relations: vec![],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![
                    PrimaryKey {
                        name: "cake_id".to_owned(),
                    },
                    PrimaryKey {
                        name: "filling_id".to_owned(),
                    },
                ],
            },
            Entity {
                table_name: "cake_filling_price".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "cake_id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "filling_id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "price".to_owned(),
                        col_type: ColumnType::Decimal(None),
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![Relation {
                    ref_table: "cake_filling".to_owned(),
                    ref_schema: None,
                    columns: vec!["cake_id".to_owned(), "filling_id".to_owned()],
                    ref_columns: vec!["cake_id".to_owned(), "filling_id".to_owned()],
                    rel_type: RelationType::BelongsTo,
                    on_delete: None,
                    on_update: None,
                    enforcement: None,
                    deferrability: None,
                    period: None,
                    self_referencing: false,
                    num_suffix: 0,
                    impl_related: true,
                }],
                conjunct_relations: vec![],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![
                    PrimaryKey {
                        name: "cake_id".to_owned(),
                    },
                    PrimaryKey {
                        name: "filling_id".to_owned(),
                    },
                ],
            },
            Entity {
                table_name: "filling".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: true,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "name".to_owned(),
                        col_type: ColumnType::String(StringLen::N(255)),
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![],
                conjunct_relations: vec![ConjunctRelation {
                    via: "cake_filling".to_owned(),
                    to: "cake".to_owned(),
                }],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![PrimaryKey {
                    name: "id".to_owned(),
                }],
            },
            Entity {
                table_name: "fruit".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: true,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "name".to_owned(),
                        col_type: ColumnType::String(StringLen::N(255)),
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "cake_id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: false,
                        identity: None,
                        not_null: false,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![
                    Relation {
                        ref_table: "cake".to_owned(),
                        ref_schema: None,
                        columns: vec!["cake_id".to_owned()],
                        ref_columns: vec!["id".to_owned()],
                        rel_type: RelationType::BelongsTo,
                        on_delete: None,
                        on_update: None,
                        enforcement: None,
                        deferrability: None,
                        period: None,
                        self_referencing: false,
                        num_suffix: 0,
                        impl_related: true,
                    },
                    Relation {
                        ref_table: "vendor".to_owned(),
                        ref_schema: None,
                        columns: vec![],
                        ref_columns: vec![],
                        rel_type: RelationType::HasMany,
                        on_delete: None,
                        on_update: None,
                        enforcement: None,
                        deferrability: None,
                        period: None,
                        self_referencing: false,
                        num_suffix: 0,
                        impl_related: true,
                    },
                ],
                conjunct_relations: vec![],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![PrimaryKey {
                    name: "id".to_owned(),
                }],
            },
            Entity {
                table_name: "vendor".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: true,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "_name_".to_owned(),
                        col_type: ColumnType::String(StringLen::N(255)),
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "fruitId".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: false,
                        identity: None,
                        not_null: false,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![Relation {
                    ref_table: "fruit".to_owned(),
                    ref_schema: None,
                    columns: vec!["fruitId".to_owned()],
                    ref_columns: vec!["id".to_owned()],
                    rel_type: RelationType::BelongsTo,
                    on_delete: None,
                    on_update: None,
                    enforcement: None,
                    deferrability: None,
                    period: None,
                    self_referencing: false,
                    num_suffix: 0,
                    impl_related: true,
                }],
                conjunct_relations: vec![],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![PrimaryKey {
                    name: "id".to_owned(),
                }],
            },
            Entity {
                table_name: "rust_keyword".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: true,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "testing".to_owned(),
                        col_type: ColumnType::SmallInteger,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "rust".to_owned(),
                        col_type: ColumnType::SmallInteger,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "keywords".to_owned(),
                        col_type: ColumnType::SmallInteger,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "type".to_owned(),
                        col_type: ColumnType::SmallInteger,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "typeof".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "crate".to_owned(),
                        col_type: ColumnType::BigInteger,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "self".to_owned(),
                        col_type: ColumnType::BigInteger,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "self_id1".to_owned(),
                        col_type: ColumnType::BigInteger,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "self_id2".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "fruit_id1".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "fruit_id2".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "cake_id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![
                    Relation {
                        ref_table: "rust_keyword".to_owned(),
                        ref_schema: None,
                        columns: vec!["self_id1".to_owned()],
                        ref_columns: vec!["id".to_owned()],
                        rel_type: RelationType::BelongsTo,
                        on_delete: None,
                        on_update: None,
                        enforcement: None,
                        deferrability: None,
                        period: None,
                        self_referencing: true,
                        num_suffix: 1,
                        impl_related: true,
                    },
                    Relation {
                        ref_table: "rust_keyword".to_owned(),
                        ref_schema: None,
                        columns: vec!["self_id2".to_owned()],
                        ref_columns: vec!["id".to_owned()],
                        rel_type: RelationType::BelongsTo,
                        on_delete: None,
                        on_update: None,
                        enforcement: None,
                        deferrability: None,
                        period: None,
                        self_referencing: true,
                        num_suffix: 2,
                        impl_related: true,
                    },
                    Relation {
                        ref_table: "fruit".to_owned(),
                        ref_schema: None,
                        columns: vec!["fruit_id1".to_owned()],
                        ref_columns: vec!["id".to_owned()],
                        rel_type: RelationType::BelongsTo,
                        on_delete: None,
                        on_update: None,
                        enforcement: None,
                        deferrability: None,
                        period: None,
                        self_referencing: false,
                        num_suffix: 1,
                        impl_related: true,
                    },
                    Relation {
                        ref_table: "fruit".to_owned(),
                        ref_schema: None,
                        columns: vec!["fruit_id2".to_owned()],
                        ref_columns: vec!["id".to_owned()],
                        rel_type: RelationType::BelongsTo,
                        on_delete: None,
                        on_update: None,
                        enforcement: None,
                        deferrability: None,
                        period: None,
                        self_referencing: false,
                        num_suffix: 2,
                        impl_related: true,
                    },
                    Relation {
                        ref_table: "cake".to_owned(),
                        ref_schema: None,
                        columns: vec!["cake_id".to_owned()],
                        ref_columns: vec!["id".to_owned()],
                        rel_type: RelationType::BelongsTo,
                        on_delete: None,
                        on_update: None,
                        enforcement: None,
                        deferrability: None,
                        period: None,
                        self_referencing: false,
                        num_suffix: 0,
                        impl_related: true,
                    },
                ],
                conjunct_relations: vec![],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![PrimaryKey {
                    name: "id".to_owned(),
                }],
            },
            Entity {
                table_name: "cake_with_float".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: true,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "name".to_owned(),
                        col_type: ColumnType::Text,
                        auto_increment: false,
                        identity: None,
                        not_null: false,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "price".to_owned(),
                        col_type: ColumnType::Float,
                        auto_increment: false,
                        identity: None,
                        not_null: false,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![Relation {
                    ref_table: "fruit".to_owned(),
                    ref_schema: None,
                    columns: vec![],
                    ref_columns: vec![],
                    rel_type: RelationType::HasMany,
                    on_delete: None,
                    on_update: None,
                    enforcement: None,
                    deferrability: None,
                    period: None,
                    self_referencing: false,
                    num_suffix: 0,
                    impl_related: true,
                }],
                conjunct_relations: vec![ConjunctRelation {
                    via: "cake_filling".to_owned(),
                    to: "filling".to_owned(),
                }],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![PrimaryKey {
                    name: "id".to_owned(),
                }],
            },
            Entity {
                table_name: "cake_with_double".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: true,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "name".to_owned(),
                        col_type: ColumnType::Text,
                        auto_increment: false,
                        identity: None,
                        not_null: false,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "price".to_owned(),
                        col_type: ColumnType::Double,
                        auto_increment: false,
                        identity: None,
                        not_null: false,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![Relation {
                    ref_table: "fruit".to_owned(),
                    ref_schema: None,
                    columns: vec![],
                    ref_columns: vec![],
                    rel_type: RelationType::HasMany,
                    on_delete: None,
                    on_update: None,
                    enforcement: None,
                    deferrability: None,
                    period: None,
                    self_referencing: false,
                    num_suffix: 0,
                    impl_related: true,
                }],
                conjunct_relations: vec![ConjunctRelation {
                    via: "cake_filling".to_owned(),
                    to: "filling".to_owned(),
                }],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![PrimaryKey {
                    name: "id".to_owned(),
                }],
            },
            Entity {
                table_name: "collection".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: true,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "integers".to_owned(),
                        col_type: ColumnType::Array(Arc::new(ColumnType::Integer)),
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "integers_opt".to_owned(),
                        col_type: ColumnType::Array(Arc::new(ColumnType::Integer)),
                        auto_increment: false,
                        identity: None,
                        not_null: false,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![],
                conjunct_relations: vec![],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![PrimaryKey {
                    name: "id".to_owned(),
                }],
            },
            Entity {
                table_name: "collection_float".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: true,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "floats".to_owned(),
                        col_type: ColumnType::Array(Arc::new(ColumnType::Float)),
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "doubles".to_owned(),
                        col_type: ColumnType::Array(Arc::new(ColumnType::Double)),
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![],
                conjunct_relations: vec![],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![PrimaryKey {
                    name: "id".to_owned(),
                }],
            },
            Entity {
                table_name: "parent".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "id1".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "id2".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![Relation {
                    ref_table: "child".to_owned(),
                    ref_schema: None,
                    columns: vec![],
                    ref_columns: vec![],
                    rel_type: RelationType::HasMany,
                    on_delete: None,
                    on_update: None,
                    enforcement: None,
                    deferrability: None,
                    period: None,
                    self_referencing: false,
                    num_suffix: 0,
                    impl_related: true,
                }],
                conjunct_relations: vec![],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![
                    PrimaryKey {
                        name: "id1".to_owned(),
                    },
                    PrimaryKey {
                        name: "id2".to_owned(),
                    },
                ],
            },
            Entity {
                table_name: "child".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: true,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "parent_id1".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "parent_id2".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![Relation {
                    ref_table: "parent".to_owned(),
                    ref_schema: None,
                    columns: vec!["parent_id1".to_owned(), "parent_id2".to_owned()],
                    ref_columns: vec!["id1".to_owned(), "id2".to_owned()],
                    rel_type: RelationType::BelongsTo,
                    on_delete: None,
                    on_update: None,
                    enforcement: None,
                    deferrability: None,
                    period: None,
                    self_referencing: false,
                    num_suffix: 0,
                    impl_related: true,
                }],
                conjunct_relations: vec![],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![PrimaryKey {
                    name: "id".to_owned(),
                }],
            },
        ]
    }

    fn parse_from_file<R>(inner: R) -> io::Result<TokenStream>
    where
        R: Read,
    {
        let mut reader = BufReader::new(inner);
        let mut lines: Vec<String> = Vec::new();

        reader.read_until(b';', &mut Vec::new())?;

        let mut line = String::new();
        while reader.read_line(&mut line)? > 0 {
            lines.push(line.to_owned());
            line.clear();
        }
        let content = lines.join("");
        Ok(content.parse().unwrap())
    }

    #[test]
    fn test_gen_compact_code_blocks() -> io::Result<()> {
        let entities = setup();
        const ENTITY_FILES: [&str; 13] = [
            include_str!("../../tests/compact/cake.rs"),
            include_str!("../../tests/compact/cake_filling.rs"),
            include_str!("../../tests/compact/cake_filling_price.rs"),
            include_str!("../../tests/compact/filling.rs"),
            include_str!("../../tests/compact/fruit.rs"),
            include_str!("../../tests/compact/vendor.rs"),
            include_str!("../../tests/compact/rust_keyword.rs"),
            include_str!("../../tests/compact/cake_with_float.rs"),
            include_str!("../../tests/compact/cake_with_double.rs"),
            include_str!("../../tests/compact/collection.rs"),
            include_str!("../../tests/compact/collection_float.rs"),
            include_str!("../../tests/compact/parent.rs"),
            include_str!("../../tests/compact/child.rs"),
        ];
        const ENTITY_FILES_WITH_SCHEMA_NAME: [&str; 13] = [
            include_str!("../../tests/compact_with_schema_name/cake.rs"),
            include_str!("../../tests/compact_with_schema_name/cake_filling.rs"),
            include_str!("../../tests/compact_with_schema_name/cake_filling_price.rs"),
            include_str!("../../tests/compact_with_schema_name/filling.rs"),
            include_str!("../../tests/compact_with_schema_name/fruit.rs"),
            include_str!("../../tests/compact_with_schema_name/vendor.rs"),
            include_str!("../../tests/compact_with_schema_name/rust_keyword.rs"),
            include_str!("../../tests/compact_with_schema_name/cake_with_float.rs"),
            include_str!("../../tests/compact_with_schema_name/cake_with_double.rs"),
            include_str!("../../tests/compact_with_schema_name/collection.rs"),
            include_str!("../../tests/compact_with_schema_name/collection_float.rs"),
            include_str!("../../tests/compact_with_schema_name/parent.rs"),
            include_str!("../../tests/compact_with_schema_name/child.rs"),
        ];

        assert_eq!(entities.len(), ENTITY_FILES.len());

        for (i, entity) in entities.iter().enumerate() {
            assert_eq!(
                parse_from_file(ENTITY_FILES[i].as_bytes())?.to_string(),
                EntityWriter::gen_compact_code_blocks(
                    entity,
                    &crate::WithSerde::None,
                    &None,
                    false,
                    false,
                    &TokenStream::new(),
                    &TokenStream::new()
                )
                .into_iter()
                .skip(1)
                .fold(TokenStream::new(), |mut acc, tok| {
                    acc.extend(tok);
                    acc
                })
                .to_string()
            );
            assert_eq!(
                parse_from_file(ENTITY_FILES_WITH_SCHEMA_NAME[i].as_bytes())?.to_string(),
                EntityWriter::gen_compact_code_blocks(
                    entity,
                    &crate::WithSerde::None,
                    &Some("schema_name".to_owned()),
                    false,
                    false,
                    &TokenStream::new(),
                    &TokenStream::new()
                )
                .into_iter()
                .skip(1)
                .fold(TokenStream::new(), |mut acc, tok| {
                    acc.extend(tok);
                    acc
                })
                .to_string()
            );
        }

        Ok(())
    }

    #[test]
    fn test_gen_with_serde() -> io::Result<()> {
        let cake_entity = setup().first().unwrap().clone();

        assert_eq!(cake_entity.get_table_name_snake_case(), "cake");

        assert_eq!(
            comparable_file_string(include_str!("../../tests/compact_with_serde/cake_none.rs"))?,
            generated_to_string(EntityWriter::gen_compact_code_blocks(
                &cake_entity,
                &WithSerde::None,
                &None,
                false,
                false,
                &TokenStream::new(),
                &TokenStream::new()
            ))
        );
        assert_eq!(
            comparable_file_string(include_str!(
                "../../tests/compact_with_serde/cake_serialize.rs"
            ))?,
            generated_to_string(EntityWriter::gen_compact_code_blocks(
                &cake_entity,
                &WithSerde::Serialize,
                &None,
                false,
                false,
                &TokenStream::new(),
                &TokenStream::new()
            ))
        );
        assert_eq!(
            comparable_file_string(include_str!(
                "../../tests/compact_with_serde/cake_deserialize.rs"
            ))?,
            generated_to_string(EntityWriter::gen_compact_code_blocks(
                &cake_entity,
                &WithSerde::Deserialize,
                &None,
                true,
                false,
                &TokenStream::new(),
                &TokenStream::new()
            ))
        );
        assert_eq!(
            comparable_file_string(include_str!("../../tests/compact_with_serde/cake_both.rs"))?,
            generated_to_string(EntityWriter::gen_compact_code_blocks(
                &cake_entity,
                &WithSerde::Both,
                &None,
                true,
                false,
                &TokenStream::new(),
                &TokenStream::new()
            ))
        );

        Ok(())
    }

    #[test]
    fn test_gen_with_derives() -> io::Result<()> {
        let mut cake_entity = setup().get_mut(0).unwrap().clone();

        assert_eq!(cake_entity.get_table_name_snake_case(), "cake");

        assert_eq!(
            comparable_file_string(include_str!(
                "../../tests/compact_with_derives/cake_none.rs"
            ))?,
            generated_to_string(EntityWriter::gen_compact_code_blocks(
                &cake_entity,
                &WithSerde::None,
                &None,
                false,
                false,
                &TokenStream::new(),
                &TokenStream::new()
            ))
        );
        assert_eq!(
            comparable_file_string(include_str!("../../tests/compact_with_derives/cake_one.rs"))?,
            generated_to_string(EntityWriter::gen_compact_code_blocks(
                &cake_entity,
                &WithSerde::None,
                &None,
                false,
                false,
                &bonus_derive("model_extra_derives", ["ts_rs::TS"]).expect("valid derives"),
                &TokenStream::new()
            ))
        );
        assert_eq!(
            comparable_file_string(include_str!(
                "../../tests/compact_with_derives/cake_multiple.rs"
            ))?,
            generated_to_string(EntityWriter::gen_compact_code_blocks(
                &cake_entity,
                &WithSerde::None,
                &None,
                false,
                false,
                &bonus_derive("model_extra_derives", ["ts_rs::TS", "utoipa::ToSchema"])
                    .expect("valid derives"),
                &TokenStream::new()
            ))
        );

        // Make the `name` column of `cake` entity as hidden column
        cake_entity.columns[1].name = "_name".into();

        assert_serde_variant_results(
            &cake_entity,
            &(
                include_str!("../../tests/compact_with_serde/cake_serialize_with_hidden_column.rs"),
                WithSerde::Serialize,
                None,
            ),
            Box::new(EntityWriter::gen_compact_code_blocks),
        )?;

        Ok(())
    }

    #[allow(clippy::type_complexity)]
    fn assert_serde_variant_results(
        cake_entity: &Entity,
        entity_serde_variant: &(&str, WithSerde, Option<String>),
        generator: Box<
            dyn Fn(
                &Entity,
                &WithSerde,
                &Option<String>,
                bool,
                bool,
                &TokenStream,
                &TokenStream,
            ) -> Vec<TokenStream>,
        >,
    ) -> io::Result<()> {
        let mut reader = BufReader::new(entity_serde_variant.0.as_bytes());
        let mut lines: Vec<String> = Vec::new();
        let serde_skip_deserializing_primary_key = matches!(
            entity_serde_variant.1,
            WithSerde::Both | WithSerde::Deserialize
        );
        let serde_skip_hidden_column = matches!(entity_serde_variant.1, WithSerde::Serialize);

        let mut line = String::new();
        while reader.read_line(&mut line)? > 0 {
            lines.push(line.to_owned());
            line.clear();
        }
        let content = lines.join("");
        let expected: TokenStream = content.parse().unwrap();
        println!("{:?}", entity_serde_variant.1);
        let generated = generator(
            cake_entity,
            &entity_serde_variant.1,
            &entity_serde_variant.2,
            serde_skip_deserializing_primary_key,
            serde_skip_hidden_column,
            &TokenStream::new(),
            &TokenStream::new(),
        )
        .into_iter()
        .fold(TokenStream::new(), |mut acc, tok| {
            acc.extend(tok);
            acc
        });

        assert_eq!(expected.to_string(), generated.to_string());
        Ok(())
    }

    #[test]
    fn test_gen_with_attributes() -> io::Result<()> {
        let cake_entity = setup().first().unwrap().clone();

        assert_eq!(cake_entity.get_table_name_snake_case(), "cake");

        assert_eq!(
            comparable_file_string(include_str!(
                "../../tests/compact_with_attributes/cake_none.rs"
            ))?,
            generated_to_string(EntityWriter::gen_compact_code_blocks(
                &cake_entity,
                &WithSerde::None,
                &None,
                false,
                false,
                &TokenStream::new(),
                &TokenStream::new()
            ))
        );
        assert_eq!(
            comparable_file_string(include_str!(
                "../../tests/compact_with_attributes/cake_one.rs"
            ))?,
            generated_to_string(EntityWriter::gen_compact_code_blocks(
                &cake_entity,
                &WithSerde::None,
                &None,
                false,
                false,
                &TokenStream::new(),
                &bonus_attributes(
                    "model_extra_attributes",
                    [r#"serde(rename_all = "camelCase")"#]
                )
                .expect("valid attributes")
            ))
        );
        assert_eq!(
            comparable_file_string(include_str!(
                "../../tests/compact_with_attributes/cake_multiple.rs"
            ))?,
            generated_to_string(EntityWriter::gen_compact_code_blocks(
                &cake_entity,
                &WithSerde::None,
                &None,
                false,
                false,
                &TokenStream::new(),
                &bonus_attributes(
                    "model_extra_attributes",
                    [r#"serde(rename_all = "camelCase")"#, "ts(export)"]
                )
                .expect("valid attributes")
            ))
        );

        Ok(())
    }

    fn generated_to_string(generated: Vec<TokenStream>) -> String {
        generated
            .into_iter()
            .fold(TokenStream::new(), |mut acc, tok| {
                acc.extend(tok);
                acc
            })
            .to_string()
    }

    fn comparable_file_string(file: &str) -> io::Result<String> {
        let mut reader = BufReader::new(file.as_bytes());
        let mut lines: Vec<String> = Vec::new();

        let mut line = String::new();
        while reader.read_line(&mut line)? > 0 {
            lines.push(line.to_owned());
            line.clear();
        }
        let content = lines.join("");
        let expected: TokenStream = content.parse().unwrap();

        Ok(expected.to_string())
    }

    #[test]
    fn test_gen_postgres() -> io::Result<()> {
        let entities = [
            // This tests that the JsonBinary column type is annotated
            // correctly in compact entity form. More information can be found
            // in SeaORM's issue 1344:
            //
            // https://github.com/SeaQL/sea-orm/issues/1344
            Entity {
                table_name: "task".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: true,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "payload".to_owned(),
                        col_type: ColumnType::Json,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "payload_binary".to_owned(),
                        col_type: ColumnType::JsonBinary,
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![],
                conjunct_relations: vec![],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![PrimaryKey {
                    name: "id".to_owned(),
                }],
            },
        ];
        const ENTITY_FILES: [&str; 1] = [include_str!("../../tests/postgres/binary_json.rs")];

        assert_eq!(entities.len(), ENTITY_FILES.len());

        for (i, entity) in entities.iter().enumerate() {
            assert_eq!(
                parse_from_file(ENTITY_FILES[i].as_bytes())?.to_string(),
                EntityWriter::gen_compact_code_blocks(
                    entity,
                    &crate::WithSerde::None,
                    &None,
                    false,
                    false,
                    &TokenStream::new(),
                    &TokenStream::new()
                )
                .into_iter()
                .skip(1)
                .fold(TokenStream::new(), |mut acc, tok| {
                    acc.extend(tok);
                    acc
                })
                .to_string()
            );
        }

        Ok(())
    }

    #[test]
    fn test_gen_import_active_enum() -> io::Result<()> {
        let entities = [
            Entity {
                table_name: "tea_pairing".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: true,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "first_tea".to_owned(),
                        col_type: ColumnType::Enum {
                            schema: None,
                            name: Name::runtime("tea_enum"),
                            variants: vec![
                                Name::runtime("everyday_tea"),
                                Name::runtime("breakfast_tea"),
                            ],
                        },
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "second_tea".to_owned(),
                        col_type: ColumnType::Enum {
                            schema: None,
                            name: Name::runtime("tea_enum"),
                            variants: vec![
                                Name::runtime("everyday_tea"),
                                Name::runtime("breakfast_tea"),
                            ],
                        },
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![],
                conjunct_relations: vec![],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![PrimaryKey {
                    name: "id".to_owned(),
                }],
            },
            Entity {
                table_name: "tea_pairing_with_size".to_owned(),
                schema_name: None,
                columns: vec![
                    Column {
                        name: "id".to_owned(),
                        col_type: ColumnType::Integer,
                        auto_increment: true,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "first_tea".to_owned(),
                        col_type: ColumnType::Enum {
                            schema: None,
                            name: Name::runtime("tea_enum"),
                            variants: vec![
                                Name::runtime("everyday_tea"),
                                Name::runtime("breakfast_tea"),
                            ],
                        },
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "second_tea".to_owned(),
                        col_type: ColumnType::Enum {
                            schema: None,
                            name: Name::runtime("tea_enum"),
                            variants: vec![
                                Name::runtime("everyday_tea"),
                                Name::runtime("breakfast_tea"),
                            ],
                        },
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                    Column {
                        name: "size".to_owned(),
                        col_type: ColumnType::Enum {
                            schema: None,
                            name: Name::runtime("tea_size"),
                            variants: vec![
                                Name::runtime("small"),
                                Name::runtime("medium"),
                                Name::runtime("huge"),
                            ],
                        },
                        auto_increment: false,
                        identity: None,
                        not_null: true,
                        unique: false,
                        default: None,
                        generated: None,
                        enum_home: None,
                    },
                ],
                relations: vec![],
                conjunct_relations: vec![],
                without_overlaps: false,
                disjoint: None,
                one_of: Vec::new(),
                unique_keys: Vec::new(),
                primary_keys: vec![PrimaryKey {
                    name: "id".to_owned(),
                }],
            },
        ];

        assert_eq!(
            quote!(
                use super::pgorm_active_enums::TeaEnum;
            )
            .to_string(),
            EntityWriter::gen_import_active_enum(&entities[0]).to_string()
        );

        assert_eq!(
            quote!(
                use super::pgorm_active_enums::TeaEnum;
                use super::pgorm_active_enums::TeaSize;
            )
            .to_string(),
            EntityWriter::gen_import_active_enum(&entities[1]).to_string()
        );

        Ok(())
    }
}
