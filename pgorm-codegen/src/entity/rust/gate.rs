//! The Rust writer's gate: what the entities a schema creates say that Rust
//! cannot, refused while the caller can still be handed the failure, so the
//! writer meets only entities it writes as Rust that compiles.

use super::disjoint::active_model_ident;
use super::ident::safe_ident;
use crate::{
    ActiveEnum, CreatedRangeType, Entities, Entity, EntityTransformer, EntityWriter,
    EntityWriterContext, Error, KindType, WithSerde, entity::disjoint::kind_type_ident,
    util::escape_rust_keyword,
};
use heck::{ToSnakeCase, ToUpperCamelCase};
use pgorm_query::TableCreateStatement;
use proc_macro2::{TokenStream, TokenTree};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The most columns an entity's primary key can have: its `ValueType` is a
/// tuple for a composite key, and pgorm's key traits stop at 12 parts.
// [spec:pgorm:sem:codegen.entity.transform+18]
const MAX_KEY_COLUMNS: usize = 12;

impl EntityTransformer {
    /// The entities the tables `table_create_stmts` create, through the Rust
    /// writer's gate.
    // [spec:pgorm:sem:codegen.entity.transform+18]
    // [spec:pgorm:sem:codegen.entity.transform.inverse+2]
    // [spec:pgorm:sem:codegen.entity.transform.conjunct+2]
    // [spec:pgorm:req:codegen.entity.collisions+3]
    // [spec:pgorm:def:codegen.entity+9]
    // [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1]
    pub fn transform(
        table_create_stmts: Vec<TableCreateStatement>,
        context: &EntityWriterContext,
    ) -> Result<EntityWriter, Error> {
        EntityWriter::new(Self::entities(
            table_create_stmts,
            &context.unkinded_disjoint,
        )?)
    }
}

impl EntityWriter {
    /// The writer of `entities`, once nothing they say is beyond Rust: a DB
    /// name whose Rust form is no identifier, two names deriving one
    /// identifier, a type the generated Rust can carry under one schema only,
    /// a key wider than its tuple. The enums a disjoint entity's derive
    /// declares, its kind types, are left to it.
    // [spec:pgorm:sem:codegen.entity.transform+18]
    // [spec:pgorm:req:codegen.entity.collisions+3]
    // [spec:pgorm:sem:codegen.entity.keywords+2]
    // [spec:pgorm:req:codegen.features]
    pub(crate) fn new(entities: Entities) -> Result<Self, Error> {
        let Entities {
            entities,
            mut enums,
            ranges,
            kind_types,
        } = entities;
        for entity in entities.iter() {
            let table = entity.ident().to_string();
            for column in entity.columns.iter() {
                column
                    .validate_idents()
                    .map_err(|err| err.in_table(&table))?;
            }
        }
        refuse_two_qualifications(&enums, &ranges)?;
        for entity in entities.iter() {
            refuse_wide_key(entity)?;
        }
        for entity in entities.iter() {
            refuse_unit_variant(entity)?;
        }
        validate_identities(&entities, &enums, &ranges, &kind_types)?;
        // The clash checks below read the names the writer's code uses, and
        // building that code needs every name an identifier.
        for entity in entities.iter() {
            entity.validate_idents()?;
        }
        refuse_disjoint_name_clashes(&entities, &enums, &ranges, &kind_types)?;
        refuse_one_of_name_clashes(
            &entities,
            &generated_type_names(&enums, &ranges, &kind_types),
        )?;
        for active_enum in enums.values() {
            active_enum.validate()?;
        }
        // A kind type is declared by its disjoint entity's derive.
        enums.retain(|key, _| !kind_types.contains_key(key));
        Ok(Self {
            entities,
            enums,
            ranges,
        })
    }
}

/// An enum or a range type named the same under two qualifications: the
/// Rust enum or newtype generated for it is named for the bare name, and
/// carries one schema.
// [spec:pgorm:req:codegen.entity.collisions+3]
// [spec:pgorm:sem:codegen.entity.enums+10]
fn refuse_two_qualifications(
    enums: &BTreeMap<String, ActiveEnum>,
    ranges: &BTreeMap<String, CreatedRangeType>,
) -> Result<(), Error> {
    let mut names = BTreeSet::new();
    for active_enum in enums.values() {
        let name = active_enum.enum_name.to_string();
        if !names.insert(name.clone()) {
            return Err(Error::TransformError(format!(
                "enum `{name}` is used under two qualifications; the generated \
                 Rust enum can carry only one"
            )));
        }
    }
    let mut names = BTreeSet::new();
    for range in ranges.values() {
        let name = range.name.to_string();
        if !names.insert(name.clone()) {
            return Err(Error::TransformError(format!(
                "range type `{name}` is used under two qualifications; the generated \
                 Rust newtype can carry only one"
            )));
        }
    }
    Ok(())
}

/// A primary key wider than [`MAX_KEY_COLUMNS`], which no key tuple holds.
// [spec:pgorm:sem:codegen.entity.transform+18]
fn refuse_wide_key(entity: &Entity) -> Result<(), Error> {
    if entity.primary_keys.len() > MAX_KEY_COLUMNS {
        return Err(Error::TransformError(format!(
            "table `{}`: a primary key of {} columns; an entity's key has at \
             most {MAX_KEY_COLUMNS}, the widest tuple its value can be",
            entity.ident(),
            entity.primary_keys.len()
        )));
    }
    Ok(())
}

/// The unit variant of an unkinded disjoint entity, the one kind with no
/// table, named as no Rust identifier.
// [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1]
fn refuse_unit_variant(entity: &Entity) -> Result<(), Error> {
    let Some(disjoint) = entity
        .disjoint
        .as_ref()
        .filter(|disjoint| disjoint.kinded.is_none())
    else {
        return Ok(());
    };
    for kind in disjoint.kinds.iter().filter(|kind| kind.table.is_none()) {
        safe_ident("its unit variant", &kind.variant).map_err(|error| {
            let why = match error {
                Error::TransformError(message) => message,
                other => other.to_string(),
            };
            Error::TransformError(format!(
                "unkinded disjoint entity over `{}`: {why}",
                disjoint.named
            ))
        })?;
    }
    Ok(())
}

/// Every identity the writer derives from a DB name belongs to one name alone.
/// Two tables that derive one module name would write one file over the other
/// and declare the module twice; two columns of a table, two enums or two
/// values of one enum that derive one identifier would each emit a duplicate
/// definition.
// [spec:pgorm:req:codegen.entity.collisions+3]
// [spec:pgorm:sem:codegen.entity.disjoint+3]    a kind type claims the name its derive gives it
fn validate_identities(
    entities: &[Entity],
    enums: &BTreeMap<String, ActiveEnum>,
    ranges: &BTreeMap<String, CreatedRangeType>,
    kind_types: &BTreeMap<String, KindType>,
) -> Result<(), Error> {
    let mut modules = Claims::default();
    let mut types = Claims::default();
    for entity in entities.iter() {
        let table_name = entity.ident().to_string();
        modules.claim(
            "tables",
            "module name",
            escape_rust_keyword(entity.get_table_name_snake_case()),
            &table_name,
        )?;
        types.claim(
            "tables",
            "type name",
            escape_rust_keyword(entity.get_table_name_camel_case()),
            &table_name,
        )?;
        let mut fields = Claims::default();
        let whose = format!("table `{table_name}` columns");
        for column in entity.columns.iter() {
            fields.claim(
                &whose,
                "field name",
                escape_rust_keyword(column.name.to_snake_case()),
                &column.name,
            )?;
        }
    }
    let mut enum_types = Claims::default();
    for (enum_name, active_enum) in enums.iter() {
        // The name the writer declares, its schema no part of it: the
        // qualified key would claim `MarketGrade` for `market.grade`, which
        // declares `Grade`.
        let type_name = match kind_types.get(enum_name) {
            Some(kind_type) => kind_type.rust_name.clone(),
            None => active_enum.enum_name.to_string().to_upper_camel_case(),
        };
        enum_types.claim("enums", "type name", type_name, enum_name)?;
        let mut variants = Claims::default();
        let whose = format!("enum `{enum_name}` values");
        for (value, variant) in active_enum.variant_names() {
            variants.claim(&whose, "variant name", variant, &value)?;
        }
    }
    for (key, range) in ranges.iter() {
        enum_types.claim(
            "enums and range types",
            "type name",
            range.name.to_string().to_upper_camel_case(),
            key,
        )?;
    }
    Ok(())
}

/// The items every entity's module declares, which no generated enum's name
/// may take.
const MODULE_ITEMS: [&str; 6] = [
    "Model",
    "Entity",
    "Column",
    "PrimaryKey",
    "ActiveModel",
    "Relation",
];

/// The Rust name of every type a schema's enums and range types generate,
/// each with what generates it.
// [spec:pgorm:sem:codegen.entity.disjoint+3]
// [spec:pgorm:def:disjoint.one-of+4]
fn generated_type_names(
    enums: &BTreeMap<String, ActiveEnum>,
    ranges: &BTreeMap<String, CreatedRangeType>,
    kind_types: &BTreeMap<String, KindType>,
) -> BTreeMap<String, String> {
    let mut type_names: BTreeMap<String, String> = BTreeMap::new();
    for (key, active_enum) in enums.iter() {
        let name = match kind_types.get(key) {
            Some(kind_type) => kind_type.rust_name.clone(),
            None => active_enum.enum_name.to_string().to_upper_camel_case(),
        };
        type_names.insert(name, format!("enum `{key}`"));
    }
    for (key, range) in ranges.iter() {
        type_names.insert(
            range.name.to_string().to_upper_camel_case(),
            format!("range type `{key}`"),
        );
    }
    type_names
}

/// The items each one-of reference's derive declares in its table's module —
/// the enum, its `<Enum>Key` and `<Enum>Model` — clash with none of the
/// module's own, its disjoint entity's, an earlier one-of reference's, a type
/// another generated item names, or a name the module's code means
/// otherwise; and no two of its variants share a name.
// [spec:pgorm:def:disjoint.one-of+4]
fn refuse_one_of_name_clashes(
    entities: &[Entity],
    type_names: &BTreeMap<String, String>,
) -> Result<(), Error> {
    for entity in entities.iter().filter(|entity| !entity.one_of.is_empty()) {
        let ident = entity.ident();
        let used = names_used(entity);
        let mut declared: BTreeMap<String, String> = BTreeMap::new();
        if let Some(disjoint) = &entity.disjoint {
            let name = entity.get_table_name_camel_case_ident().to_string();
            declared.insert(name.clone(), "its disjoint entity's enum".to_owned());
            declared.insert(
                kind_type_ident(&name),
                "its disjoint entity's kind enum".to_owned(),
            );
            if disjoint.kinded.is_some() {
                declared.insert(
                    active_model_ident(&name),
                    "its disjoint entity's active model".to_owned(),
                );
            }
        }
        for one_of in &entity.one_of {
            one_of.refuse_variant_clashes(&ident)?;
            for (item, what) in one_of.items() {
                let clash = if MODULE_ITEMS.contains(&item.as_str()) {
                    Some(format!("the entity's own `{item}`"))
                } else if let Some(other) = declared.get(&item) {
                    Some(format!("{other} `{item}`"))
                } else if let Some(source) = type_names.get(&item) {
                    Some(format!("the type {source} generates"))
                } else if used.contains(&item) {
                    Some(format!("the `{item}` its module's code names"))
                } else {
                    None
                };
                if let Some(clash) = clash {
                    return Err(Error::TransformError(format!(
                        "table `{ident}`: its one-of reference's {what} `{item}` is named as \
                         {clash}"
                    )));
                }
            }
            for (item, what) in one_of.items() {
                declared.insert(
                    item,
                    format!("its one-of reference `{}`'s {what}", one_of.name),
                );
            }
        }
    }
    Ok(())
}

/// The items a disjoint entity's derive declares in its base's module — the
/// enum, its `<Enum>Kind` and, for a kinded entity, its `<Enum>ActiveModel`
/// — clash with none of the module's own: the items its entity declares, a
/// type it imports, or a name its code means as the prelude's or the
/// language's, which an item of that name would shadow. A kind type is meant
/// wherever the module names it, so only the other two are held to the last;
/// an unkinded entity's kind enum no column names, so it is held to them all.
// [spec:pgorm:sem:codegen.entity.disjoint+3]
// [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1]
fn refuse_disjoint_name_clashes(
    entities: &[Entity],
    enums: &BTreeMap<String, ActiveEnum>,
    ranges: &BTreeMap<String, CreatedRangeType>,
    kind_types: &BTreeMap<String, KindType>,
) -> Result<(), Error> {
    let type_names = generated_type_names(enums, ranges, kind_types);
    for entity in entities.iter() {
        let Some(disjoint) = &entity.disjoint else {
            continue;
        };
        let ident = entity.ident();
        let name = entity.get_table_name_camel_case_ident().to_string();
        let declared = match disjoint.kinded {
            Some(_) => vec![
                (name.clone(), "enum"),
                (kind_type_ident(&name), "kind type"),
                (active_model_ident(&name), "active model"),
            ],
            None => vec![
                (name.clone(), "enum"),
                (kind_type_ident(&name), "kind enum"),
            ],
        };
        let used = names_used(entity);
        for (item, what) in declared.iter() {
            let clash = if MODULE_ITEMS.contains(&item.as_str()) {
                Some(format!("the entity's own `{item}`"))
            } else if let Some(source) = type_names.get(item).filter(|_| *what != "kind type") {
                Some(format!("the type {source} generates"))
            } else if *what != "kind type" && used.contains(item) {
                Some(format!("the `{item}` its module's code names"))
            } else {
                None
            };
            if let Some(clash) = clash {
                return Err(Error::TransformError(format!(
                    "table `{ident}`: its disjoint entity's {what} `{item}` is named as {clash}"
                )));
            }
        }
    }
    Ok(())
}

/// Every name the entity file of `entity` looks up in its module besides
/// its disjoint enum, under every option that adds names of its own: the
/// first segment of each path its imports, `Model`, relation attributes and
/// `Related` impls write. A later segment is looked up in the one before it,
/// and a `Relation` variant is declared in its enum, so neither can be
/// shadowed by an item of the module. Nor can a `derive(..)` list's names,
/// which are looked up among macros, where no item of the module is: a base
/// table named `debug` declares an enum `Debug` beside `#[derive(Debug)]`.
// [spec:pgorm:sem:codegen.entity.disjoint+3]
fn names_used(entity: &Entity) -> BTreeSet<String> {
    fn collect(stream: TokenStream, names: &mut BTreeSet<String>) {
        let mut colons = 0;
        let mut derive = false;
        for tree in stream {
            match &tree {
                TokenTree::Group(_) if derive => {}
                TokenTree::Ident(ident) if colons < 2 => {
                    names.insert(ident.to_string());
                }
                TokenTree::Group(group) => collect(group.stream(), names),
                _ => {}
            }
            derive = matches!(&tree, TokenTree::Ident(ident) if ident == "derive");
            colons = match &tree {
                TokenTree::Punct(punct) if punct.as_char() == ':' => colons + 1,
                _ => 0,
            };
        }
    }
    let mut names: BTreeSet<String> = ["Serialize", "Deserialize", "serde"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let mut blocks = vec![
        EntityWriter::gen_import(&WithSerde::None),
        EntityWriter::gen_compact_model_struct(
            entity,
            &WithSerde::None,
            &None,
            false,
            false,
            &TokenStream::new(),
            &TokenStream::new(),
        ),
    ];
    blocks.extend(entity.get_relation_attrs());
    blocks.extend(EntityWriter::gen_impl_conjunct_related(entity));
    for block in blocks {
        collect(block, &mut names);
    }
    names
}

/// The DB name that claimed each derived identifier, so a second claim on one
/// can name both sides.
// [spec:pgorm:req:codegen.entity.collisions+3]
#[derive(Default)]
struct Claims(HashMap<String, String>);

impl Claims {
    /// Record `source`'s claim on `derived`, or report the name that claimed it
    /// first. `whose` opens the message with the names in play (`tables`,
    /// ``table `cake` columns``) and `what` names the identifier they share.
    fn claim(
        &mut self,
        whose: &str,
        what: &str,
        derived: String,
        source: &str,
    ) -> Result<(), Error> {
        if let Some(first) = self.0.insert(derived.clone(), source.to_owned()) {
            return Err(Error::TransformError(format!(
                "{whose} `{first}` and `{source}` both generate the {what} `{derived}`"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pgorm::Schema;
    use pgorm_query::TableCreateStatement;
    use std::collections::HashMap;
    use std::error::Error;

    #[test]
    fn duplicated_many_to_many_paths() -> Result<(), Box<dyn Error>> {
        use crate::tests_cfg::duplicated_many_to_many_paths::*;
        let schema = Schema::new();

        validate_compact_entities(
            vec![
                schema.create_table_from_entity(bills::Entity)?,
                schema.create_table_from_entity(users::Entity)?,
                schema.create_table_from_entity(users_saved_bills::Entity)?,
                schema.create_table_from_entity(users_votes::Entity)?,
            ],
            vec![
                (
                    "bills",
                    "src/tests_cfg/duplicated_many_to_many_paths/bills.rs",
                ),
                (
                    "users",
                    "src/tests_cfg/duplicated_many_to_many_paths/users.rs",
                ),
                (
                    "users_saved_bills",
                    "src/tests_cfg/duplicated_many_to_many_paths/users_saved_bills.rs",
                ),
                (
                    "users_votes",
                    "src/tests_cfg/duplicated_many_to_many_paths/users_votes.rs",
                ),
            ],
        )
    }

    #[test]
    fn many_to_many() -> Result<(), Box<dyn Error>> {
        use crate::tests_cfg::many_to_many::*;
        let schema = Schema::new();

        validate_compact_entities(
            vec![
                schema.create_table_from_entity(bills::Entity)?,
                schema.create_table_from_entity(users::Entity)?,
                schema.create_table_from_entity(users_votes::Entity)?,
            ],
            vec![
                ("bills", "src/tests_cfg/many_to_many/bills.rs"),
                ("users", "src/tests_cfg/many_to_many/users.rs"),
                ("users_votes", "src/tests_cfg/many_to_many/users_votes.rs"),
            ],
        )
    }

    #[test]
    fn many_to_many_multiple() -> Result<(), Box<dyn Error>> {
        use crate::tests_cfg::many_to_many_multiple::*;
        let schema = Schema::new();

        validate_compact_entities(
            vec![
                schema.create_table_from_entity(bills::Entity)?,
                schema.create_table_from_entity(users::Entity)?,
                schema.create_table_from_entity(users_votes::Entity)?,
            ],
            vec![
                ("bills", "src/tests_cfg/many_to_many_multiple/bills.rs"),
                ("users", "src/tests_cfg/many_to_many_multiple/users.rs"),
                (
                    "users_votes",
                    "src/tests_cfg/many_to_many_multiple/users_votes.rs",
                ),
            ],
        )
    }

    #[test]
    fn self_referencing() -> Result<(), Box<dyn Error>> {
        use crate::tests_cfg::self_referencing::*;
        let schema = Schema::new();

        validate_compact_entities(
            vec![
                schema.create_table_from_entity(bills::Entity)?,
                schema.create_table_from_entity(users::Entity)?,
            ],
            vec![
                ("bills", "src/tests_cfg/self_referencing/bills.rs"),
                ("users", "src/tests_cfg/self_referencing/users.rs"),
            ],
        )
    }

    fn validate_compact_entities(
        table_create_stmts: Vec<TableCreateStatement>,
        files: Vec<(&str, &str)>,
    ) -> Result<(), Box<dyn Error>> {
        let context = crate::EntityWriterContext::new(crate::EntityWriterOptions::default())?;
        let entities: HashMap<_, _> = EntityTransformer::transform(table_create_stmts, &context)?
            .entities
            .into_iter()
            .map(|entity| (entity.table_name.clone(), entity))
            .collect();

        for (entity_name, golden) in files {
            let entity = entities
                .get(entity_name)
                .expect("Forget to add entity to the list");
            crate::entity::rust::format::hold_golden(
                golden,
                &crate::entity::rust::format::file(EntityWriter::gen_compact_code_blocks(
                    entity,
                    &crate::WithSerde::None,
                    &None,
                    false,
                    false,
                    &Default::default(),
                    &Default::default(),
                )),
            );
        }

        Ok(())
    }
}
