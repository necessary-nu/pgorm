use super::column::under_domains;
use super::disjoint::{self, enum_key};
use crate::{
    ActiveEnum, Column, ConjunctRelation, CreatedRangeType, Entity, EntityWriter, Error, KindType,
    PrimaryKey, Relation, RelationType, TableIdent, UniqueKey, WithSerde,
    util::{escape_rust_keyword, repeated_column},
};
use heck::{ToSnakeCase, ToUpperCamelCase};
use pgorm_query::{ColumnType, GeneratedKind, TableCreateStatement};
use proc_macro2::{TokenStream, TokenTree};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The most columns an entity's primary key can have: its `ValueType` is a
/// tuple for a composite key, and pgorm's key traits stop at 12 parts.
// [spec:pgorm:sem:codegen.entity.transform+16]
const MAX_KEY_COLUMNS: usize = 12;

#[derive(Clone, Debug)]
pub struct EntityTransformer;

impl EntityTransformer {
    // [spec:pgorm:sem:codegen.entity.transform+16]
    // [spec:pgorm:sem:codegen.entity.transform.inverse+1]
    // [spec:pgorm:sem:codegen.entity.transform.conjunct+2]
    // [spec:pgorm:req:codegen.entity.collisions+2]
    pub fn transform(table_create_stmts: Vec<TableCreateStatement>) -> Result<EntityWriter, Error> {
        let declared: Vec<TableIdent> = table_create_stmts
            .iter()
            .map(|table_create| TableIdent::of(table_create.get_table_name()))
            .collect();
        validate_distinct_names(&declared)?;
        let mut enums: BTreeMap<String, ActiveEnum> = BTreeMap::new();
        let mut ranges: BTreeMap<String, CreatedRangeType> = BTreeMap::new();
        let mut entities: BTreeMap<TableIdent, Entity> = BTreeMap::new();
        let mut unique_keys: BTreeMap<TableIdent, Vec<UniqueKey>> = BTreeMap::new();
        for table_create in table_create_stmts.into_iter() {
            let ident = TableIdent::of(table_create.get_table_name());
            let table_name = ident.to_string();
            refuse_temporal_constraints(&table_create, &table_name)?;
            refuse_repeated_key_columns(&table_create, &table_name)?;
            let keys: Vec<UniqueKey> = table_create
                .get_unique_keys()
                .iter()
                .map(|key| UniqueKey {
                    columns: key
                        .get_columns()
                        .iter()
                        .map(|name| name.to_string())
                        .collect(),
                    plain: !key.is_nulls_not_distinct()
                        && key.get_include().is_empty()
                        && key.get_deferrability().is_none(),
                })
                .collect();
            // A temporal key's period is its last column.
            let primary_keys: Vec<PrimaryKey> = table_create
                .get_primary_key()
                .map(|key| {
                    key.get_columns()
                        .iter()
                        .chain(key.get_without_overlaps())
                        .map(|name| PrimaryKey {
                            name: name.to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let without_overlaps = table_create
                .get_primary_key()
                .is_some_and(|key| key.get_without_overlaps().is_some());
            let mut columns: Vec<Column> = Vec::new();
            for col_def in table_create.get_columns() {
                let mut col = Column::try_from(col_def).map_err(|err| err.in_table(&table_name))?;
                col.unique = keys
                    .iter()
                    .any(|key| key.columns.len() == 1 && key.columns.contains(&col.name));
                if let pgorm_query::ColumnType::Enum {
                    name,
                    schema,
                    variants,
                } = col.get_inner_col_type()
                {
                    let key = enum_key(name, schema);
                    let colliding = enums.iter().any(|(existing_key, existing)| {
                        existing.enum_name.to_string() == name.to_string() && *existing_key != key
                    });
                    if colliding {
                        return Err(Error::TransformError(format!(
                            "enum `{}` is used under two qualifications; the generated \
                             Rust enum can carry only one",
                            name.to_string()
                        )));
                    }
                    enums.insert(
                        key,
                        ActiveEnum {
                            enum_name: name.clone(),
                            schema: schema.clone(),
                            values: variants.clone(),
                        },
                    );
                }
                if let pgorm_query::ColumnType::CreatedRange {
                    name,
                    schema,
                    subtype,
                }
                | pgorm_query::ColumnType::CreatedMultirange {
                    name,
                    schema,
                    subtype,
                } = col.get_inner_col_type()
                {
                    let key = enum_key(name, schema);
                    let colliding = ranges.iter().any(|(existing_key, existing)| {
                        existing.name.to_string() == name.to_string() && *existing_key != key
                    });
                    if colliding {
                        return Err(Error::TransformError(format!(
                            "range type `{}` is used under two qualifications; the generated \
                             Rust newtype can carry only one",
                            name.to_string()
                        )));
                    }
                    ranges.insert(
                        key,
                        CreatedRangeType {
                            name: name.clone(),
                            schema: schema.clone(),
                            subtype: subtype.as_ref().clone(),
                            multirange: matches!(
                                col.get_inner_col_type(),
                                pgorm_query::ColumnType::CreatedMultirange { .. }
                            ),
                        },
                    );
                }
                columns.push(col);
            }
            let relations: Vec<Relation> = table_create
                .get_foreign_key_create_stmts()
                .iter()
                .map(|fk_create_stmt| {
                    let mut relation = Relation::from(fk_create_stmt.get_foreign_key());
                    if let Some(target) = resolve_reference(&declared, &relation.ref_ident()) {
                        relation.ref_schema = target.schema.clone();
                    }
                    relation.self_referencing = relation.ref_ident() == ident;
                    relation
                })
                .collect();
            refuse_period_before_key(&columns, &primary_keys, without_overlaps, &table_name)?;
            refuse_unwritable_expressions(&columns, &primary_keys, &table_name)?;
            if primary_keys.len() > MAX_KEY_COLUMNS {
                return Err(Error::TransformError(format!(
                    "table `{table_name}`: a primary key of {} columns; an entity's key has at \
                     most {MAX_KEY_COLUMNS}, the widest tuple its value can be",
                    primary_keys.len()
                )));
            }
            let entity = Entity {
                table_name: ident.table.clone(),
                schema_name: ident.schema.clone(),
                columns,
                relations,
                conjunct_relations: vec![],
                primary_keys,
                without_overlaps,
                disjoint: None,
            };
            entities.insert(ident.clone(), entity);
            unique_keys.insert(ident, keys);
        }
        validate_references(&entities)?;
        refuse_unkeyable_references(&entities)?;
        let kind_types = disjoint::recognise(&mut entities, &unique_keys)?;
        validate_identities(&entities, &enums, &ranges, &kind_types)?;
        refuse_disjoint_name_clashes(&entities, &enums, &ranges, &kind_types)?;
        let mut inverse_relations: BTreeMap<TableIdent, Vec<Relation>> = BTreeMap::new();
        for (ident, entity) in entities.iter_mut() {
            number_relations(&mut entity.relations);
            let unique_column_sets: Vec<BTreeSet<String>> = unique_keys
                .get(ident)
                .into_iter()
                .flatten()
                .map(|key| key.columns.iter().cloned().collect())
                .collect();
            for rel in entity.relations.iter() {
                if let Some((target, inverse)) =
                    inverse_relation(ident, entity, rel, &unique_column_sets)
                {
                    inverse_relations.entry(target).or_default().push(inverse);
                }
            }
        }
        for (tbl_name, relations) in inverse_relations.into_iter() {
            if let Some(entity) = entities.get_mut(&tbl_name) {
                for relation in relations.into_iter() {
                    let duplicate_relation = entity
                        .relations
                        .iter()
                        .any(|rel| rel.ref_table == relation.ref_table);
                    if !duplicate_relation {
                        entity.relations.push(relation);
                    }
                }
            }
        }
        let mut conjunct_relations: Vec<(TableIdent, ConjunctRelation)> = Vec::new();
        for (ident, entity) in entities.iter() {
            // only the table's own foreign keys are junction legs; the inverse
            // relations synthesised above are relations of other tables' keys
            let owned: Vec<&Relation> = entity
                .relations
                .iter()
                .filter(|rel| {
                    matches!(rel.rel_type, RelationType::BelongsTo) && !rel.columns.is_empty()
                })
                .collect();
            let [one, other] = owned.as_slice() else {
                continue;
            };
            let key_columns: BTreeSet<&String> =
                one.columns.iter().chain(other.columns.iter()).collect();
            let primary_keys: BTreeSet<&String> = entity
                .primary_keys
                .iter()
                .map(|primary_key| &primary_key.name)
                .collect();
            if key_columns != primary_keys {
                continue;
            }
            for (rel, another_rel) in [(one, other), (other, one)] {
                conjunct_relations.push((
                    rel.ref_ident(),
                    ConjunctRelation {
                        via: ident.table.clone(),
                        to: another_rel.ref_table.clone(),
                    },
                ));
            }
        }
        for (ref_table, conjunct_relation) in conjunct_relations {
            if let Some(entity) = entities.get_mut(&ref_table) {
                entity.conjunct_relations.push(conjunct_relation);
            }
        }
        let entities: Vec<Entity> = entities
            .into_values()
            .map(|mut v| {
                // Filter duplicated conjunct relations
                let duplicated_to: Vec<_> = v
                    .conjunct_relations
                    .iter()
                    .fold(HashMap::new(), |mut acc, conjunct_relation| {
                        acc.entry(conjunct_relation.to.clone())
                            .and_modify(|c| *c += 1)
                            .or_insert(1);
                        acc
                    })
                    .into_iter()
                    .filter(|(_, v)| v > &1)
                    .map(|(k, _)| k)
                    .collect();
                v.conjunct_relations
                    .retain(|conjunct_relation| !duplicated_to.contains(&conjunct_relation.to));

                // A relation to the entity a conjunct relation reaches leaves its
                // `Related` impl to the `via` one, saying `related = false`
                v.relations.iter_mut().for_each(|relation| {
                    if v.conjunct_relations
                        .iter()
                        .any(|conjunct_relation| conjunct_relation.to == relation.ref_table)
                    {
                        relation.impl_related = false;
                    }
                });

                // Sort relation vectors
                v.relations.sort_by(|a, b| a.ref_table.cmp(&b.ref_table));
                v.conjunct_relations.sort_by(|a, b| a.to.cmp(&b.to));
                v
            })
            .collect();
        for entity in entities.iter() {
            entity.validate()?;
        }
        for active_enum in enums.values() {
            active_enum.validate()?;
        }
        // A kind type is declared by its disjoint entity's derive.
        enums.retain(|key, _| !kind_types.contains_key(key));
        Ok(EntityWriter {
            entities,
            enums,
            ranges,
        })
    }
}

/// Number a table's relations to one target: a single relation to a table
/// keeps suffix 0, and several are numbered from their count down in the
/// order they are declared, so the first declared is the count and the last
/// is 1.
// [spec:pgorm:sem:codegen.entity.transform+16]
fn number_relations(relations: &mut [Relation]) {
    let mut ref_table_counts: BTreeMap<TableIdent, usize> = BTreeMap::new();
    for relation in relations.iter() {
        if let Some(count) = ref_table_counts.get_mut(&relation.ref_ident()) {
            if *count == 0 {
                *count = 1;
            }
            *count += 1;
        } else {
            ref_table_counts.insert(relation.ref_ident(), 0);
        }
    }
    for rel in relations.iter_mut() {
        if let Some(count) = ref_table_counts.get_mut(&rel.ref_ident()) {
            rel.num_suffix = *count;
            if *count > 0 {
                *count -= 1;
            }
        }
    }
}

/// The relation `rel`, a foreign key of `ident`'s table, gives the table it
/// references, and that table: `HasOne` when the key is unique and `HasMany`
/// otherwise. A self-referencing key would duplicate itself, and a suffixed
/// one has no `Related` impl to build the inverse from, so neither gives one.
// [spec:pgorm:sem:codegen.entity.transform.inverse+1]
fn inverse_relation(
    ident: &TableIdent,
    entity: &Entity,
    rel: &Relation,
    unique_column_sets: &[BTreeSet<String>],
) -> Option<(TableIdent, Relation)> {
    if rel.self_referencing || rel.num_suffix > 0 {
        return None;
    }
    let key_columns: BTreeSet<String> = rel.columns.iter().cloned().collect();
    let every_column_unique = rel.columns.iter().all(|column| {
        entity
            .columns
            .iter()
            .filter(|col| col.unique)
            .any(|col| col.name.as_str() == column)
    });
    // a unique constraint over exactly the key's columns constrains
    // the key as a whole, which one-column-at-a-time cannot see
    let key_is_unique = unique_column_sets.contains(&key_columns);
    let key_is_primary = key_columns.len() == entity.primary_keys.len()
        && entity
            .primary_keys
            .iter()
            .all(|primary_key| key_columns.contains(&primary_key.name));
    let unique = every_column_unique || key_is_unique || key_is_primary;
    let mut inverse = rel.clone();
    inverse.rel_type = if unique {
        RelationType::HasOne
    } else {
        RelationType::HasMany
    };
    inverse.ref_table = ident.table.clone();
    inverse.ref_schema = ident.schema.clone();
    inverse.columns = Vec::new();
    inverse.ref_columns = Vec::new();
    Some((rel.ref_ident(), inverse))
}

/// Which table a foreign key, index or comment names, read against the tables
/// the schema declares.
///
/// A qualified reference names exactly that table. An unqualified one names
/// the table written without a qualifier, and failing that the one table with
/// that bare name — the reading `search_path` would give it in any schema that
/// generates at all, since two tables sharing a bare name are refused before
/// this is reached (`validate_distinct_names`).
// [spec:pgorm:sem:codegen.entity.transform+16]
pub(crate) fn resolve_reference<'a>(
    declared: &'a [TableIdent],
    reference: &TableIdent,
) -> Option<&'a TableIdent> {
    if let Some(exact) = declared.iter().find(|candidate| *candidate == reference) {
        return Some(exact);
    }
    if reference.schema.is_some() {
        return None;
    }
    let mut by_name = declared
        .iter()
        .filter(|candidate| candidate.table == reference.table);
    let only = by_name.next()?;
    by_name.next().is_none().then_some(only)
}

/// A temporal unique key has no entity form: an entity's unique columns are
/// single columns, and a key ending `WITHOUT OVERLAPS` has two at least, so
/// read as its columns it would claim a uniqueness the table does not have.
/// A temporal primary key and a `PERIOD` foreign key the entity holds.
// [spec:pgorm:sem:codegen.entity.transform+16]
fn refuse_temporal_constraints(table: &TableCreateStatement, name: &str) -> Result<(), Error> {
    let temporal_unique = table
        .get_unique_keys()
        .iter()
        .any(|key| key.get_without_overlaps().is_some());
    if temporal_unique {
        return Err(Error::TransformError(format!(
            "table `{name}`: an entity cannot hold a WITHOUT OVERLAPS unique key"
        )));
    }
    Ok(())
}

/// A primary or unique key naming a column twice, which PostgreSQL refuses
/// (`42701`) and a caller-built statement can still hold. Read as written it
/// would generate a `PrimaryKey` enum with one variant twice, which does not
/// compile, and a unique key over `(a, a)` would mark `a` unique on its own;
/// so it is refused by name, never read as the key without its repeat.
// [spec:pgorm:sem:codegen.entity.transform+16]
fn refuse_repeated_key_columns(table: &TableCreateStatement, name: &str) -> Result<(), Error> {
    let primary = table
        .get_primary_key()
        .map(|key| (key.get_columns(), key.get_without_overlaps()));
    let keys = primary.into_iter().chain(
        table
            .get_unique_keys()
            .iter()
            .map(|key| (key.get_columns(), key.get_without_overlaps())),
    );
    for (columns, period) in keys {
        let columns = columns
            .iter()
            .chain(period)
            .map(|column| column.to_string());
        if let Some(column) = repeated_column(columns) {
            return Err(Error::TransformError(format!(
                "table `{name}`: a key names column `{column}` twice"
            )));
        }
    }
    Ok(())
}

/// A column expression the entity cannot be written with: a `DEFAULT`
/// reading a column (PostgreSQL refuses a column reference in one, `0A000`),
/// a generation expression reading a column the table does not have, which
/// the generated `Column::<name>` would not compile against, and a `VIRTUAL`
/// generated column in a key, which PostgreSQL does not support (`0A000`)
/// and the derive refuses.
// [spec:pgorm:sem:codegen.entity.expressions+2]
fn refuse_unwritable_expressions(
    columns: &[Column],
    primary_keys: &[PrimaryKey],
    name: &str,
) -> Result<(), Error> {
    for column in columns {
        let keyed = column.unique || primary_keys.iter().any(|key| key.name == column.name);
        if keyed && matches!(column.generated, Some((_, GeneratedKind::Virtual))) {
            return Err(Error::TransformError(format!(
                "table `{name}` column `{}`: a VIRTUAL generated column cannot be keyed, \
                 which PostgreSQL does not support",
                column.name
            )));
        }
        if let Some(read) = column.default.iter().flat_map(|expr| expr.columns()).next() {
            return Err(Error::TransformError(format!(
                "table `{name}` column `{}`: its DEFAULT reads column `{read}`; a DEFAULT \
                 reads no column",
                column.name
            )));
        }
        let generated = column.generated.iter().flat_map(|(expr, _)| expr.columns());
        for read in generated {
            if !columns.iter().any(|other| other.name == read) {
                return Err(Error::TransformError(format!(
                    "table `{name}` column `{}`: its generation expression reads column \
                     `{read}`, which the table does not have",
                    column.name
                )));
            }
        }
    }
    Ok(())
}

/// A temporal key whose period column the table declares before another key
/// column: an entity's key is its key fields in the order they are written,
/// and the derive takes `WITHOUT OVERLAPS` on the last of them alone, so the
/// generated entity would not compile.
// [spec:pgorm:sem:codegen.entity.transform+16]
fn refuse_period_before_key(
    columns: &[Column],
    primary_keys: &[PrimaryKey],
    without_overlaps: bool,
    name: &str,
) -> Result<(), Error> {
    let position = |key: &PrimaryKey| columns.iter().position(|column| column.name == key.name);
    let Some((period, rest)) = primary_keys.split_last().filter(|_| without_overlaps) else {
        return Ok(());
    };
    if rest.iter().any(|key| position(key) > position(period)) {
        return Err(Error::TransformError(format!(
            "table `{name}`: the period `{}` of a WITHOUT OVERLAPS key is declared before \
             another key column; an entity's key follows its fields' order, its period last",
            period.name
        )));
    }
    Ok(())
}

/// One table per bare name. Two tables that share one would be written to a
/// single file, and an unqualified reference to that name could mean either;
/// the schemas are what tells them apart, and a generation run has one output
/// directory to tell them apart in.
// [spec:pgorm:req:codegen.entity.collisions+2]
fn validate_distinct_names(declared: &[TableIdent]) -> Result<(), Error> {
    let mut claimed: HashMap<&str, &TableIdent> = HashMap::new();
    for ident in declared.iter() {
        let Some(first) = claimed.insert(ident.table.as_str(), ident) else {
            continue;
        };
        if first == ident {
            return Err(Error::TransformError(format!(
                "table `{ident}` is declared twice"
            )));
        }
        let module = escape_rust_keyword(ident.table.to_snake_case());
        return Err(Error::TransformError(format!(
            "tables `{first}` and `{ident}` both generate the module name `{module}`: same-named \
             tables in different schemas are different tables, and need one generation run each"
        )));
    }
    Ok(())
}

/// Every relation joins tables and columns this schema has: a generated file
/// names its target's module and columns, so a foreign key onto a table the
/// caller did not pass would generate Rust that does not compile.
// [spec:pgorm:sem:codegen.entity.transform+16]
fn validate_references(entities: &BTreeMap<TableIdent, Entity>) -> Result<(), Error> {
    for (table_name, entity) in entities.iter() {
        for relation in entity.relations.iter() {
            let ref_table = relation.ref_ident();
            let context = format!("table `{table_name}`: relation to `{ref_table}`");
            let Some(ref_entity) = entities.get(&ref_table) else {
                return Err(Error::TransformError(format!(
                    "{context} names a table the schema does not define"
                )));
            };
            let period = relation.period.as_ref();
            for column in relation
                .columns
                .iter()
                .chain(period.map(|(column, _)| column))
            {
                if !entity.columns.iter().any(|col| &col.name == column) {
                    return Err(Error::TransformError(format!(
                        "{context} constrains column `{column}`, which the table does not have"
                    )));
                }
            }
            let ref_period = period.map(|(_, ref_column)| ref_column);
            for ref_column in relation.ref_columns.iter().chain(ref_period) {
                if !ref_entity.columns.iter().any(|col| &col.name == ref_column) {
                    return Err(Error::TransformError(format!(
                        "{context} references column `{ref_column}`, which `{ref_table}` does not \
                         have"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// A foreign key PostgreSQL cannot implement: one matching a column typed as
/// a domain over an enum, on either side, which the server refuses (`42804`)
/// whatever the other side is. DDL text is never run, so the server's refusal
/// cannot have kept one out; generated, it would be a relation
/// `Schema::create_table_from_entity` refuses in turn.
// [spec:pgorm:sem:codegen.entity.transform+16]
fn refuse_unkeyable_references(entities: &BTreeMap<TableIdent, Entity>) -> Result<(), Error> {
    for (table_name, entity) in entities.iter() {
        for relation in entity.relations.iter() {
            let Some(ref_entity) = entities.get(&relation.ref_ident()) else {
                continue;
            };
            let pairs = relation.columns.iter().zip(relation.ref_columns.iter());
            for (column, ref_column) in pairs {
                if over_enum_domain(entity, column) || over_enum_domain(ref_entity, ref_column) {
                    return Err(Error::TransformError(format!(
                        "table `{table_name}`: relation to `{}` matches column `{column}` with \
                         `{ref_column}`, and PostgreSQL implements no foreign key with a column \
                         typed as a domain over an enum on either side (42804)",
                        relation.ref_ident()
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Whether `entity`'s column `name` is typed as a domain over an enum,
/// through any further domains; a domain over an array of one is keyed.
// [spec:pgorm:sem:codegen.entity.transform+16]
fn over_enum_domain(entity: &Entity, name: &str) -> bool {
    entity
        .columns
        .iter()
        .find(|column| column.name == name)
        .is_some_and(|column| match &column.col_type {
            ColumnType::Domain { base, .. } => {
                matches!(under_domains(base), ColumnType::Enum { .. })
            }
            _ => false,
        })
}

/// Every identity the writer derives from a DB name belongs to one name alone.
/// Two tables that derive one module name would write one file over the other
/// and declare the module twice; two columns of a table, two enums or two
/// values of one enum that derive one identifier would each emit a duplicate
/// definition.
// [spec:pgorm:req:codegen.entity.collisions+2]
// [spec:pgorm:sem:codegen.entity.disjoint+1]    a kind type claims the name its derive gives it
fn validate_identities(
    entities: &BTreeMap<TableIdent, Entity>,
    enums: &BTreeMap<String, ActiveEnum>,
    ranges: &BTreeMap<String, CreatedRangeType>,
    kind_types: &BTreeMap<String, KindType>,
) -> Result<(), Error> {
    let mut modules = Claims::default();
    let mut types = Claims::default();
    for (ident, entity) in entities.iter() {
        let table_name = ident.to_string();
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
        let type_name = match kind_types.get(enum_name) {
            Some(kind_type) => kind_type.rust_name.clone(),
            None => enum_name.to_upper_camel_case(),
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

/// The items a disjoint entity's derive declares in its base's module — the
/// enum, its `<Enum>Kind` and its `<Enum>ActiveModel` — clash with none of
/// the module's own: the items its entity declares, a type it imports, or a
/// name its code means as the prelude's or the language's, which an item of
/// that name would shadow. The kind type is meant wherever the module names
/// it, so only the other two are held to the last.
// [spec:pgorm:sem:codegen.entity.disjoint+1]
fn refuse_disjoint_name_clashes(
    entities: &BTreeMap<TableIdent, Entity>,
    enums: &BTreeMap<String, ActiveEnum>,
    ranges: &BTreeMap<String, CreatedRangeType>,
    kind_types: &BTreeMap<String, KindType>,
) -> Result<(), Error> {
    const MODULE_ITEMS: [&str; 6] = [
        "Model",
        "Entity",
        "Column",
        "PrimaryKey",
        "ActiveModel",
        "Relation",
    ];
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
    for (ident, entity) in entities.iter() {
        if entity.disjoint.is_none() {
            continue;
        }
        let name = entity.get_table_name_camel_case_ident().to_string();
        let declared = [
            (name.clone(), "enum"),
            (disjoint::kind_type_ident(&name), "kind type"),
            (disjoint::active_model_ident(&name), "active model"),
        ];
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
/// shadowed by an item of the module.
fn names_used(entity: &Entity) -> BTreeSet<String> {
    fn collect(stream: TokenStream, names: &mut BTreeSet<String>) {
        let mut colons = 0;
        for tree in stream {
            match &tree {
                TokenTree::Ident(ident) if colons < 2 => {
                    names.insert(ident.to_string());
                }
                TokenTree::Group(group) => collect(group.stream(), names),
                _ => {}
            }
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
        EntityWriter::gen_import_active_enum(entity),
        EntityWriter::gen_import_range_types(entity),
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
// [spec:pgorm:req:codegen.entity.collisions+2]
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
    use pretty_assertions::assert_eq;
    use proc_macro2::TokenStream;
    use std::{
        error::Error,
        io::{self, BufRead, BufReader},
    };

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
                    include_str!("../tests_cfg/duplicated_many_to_many_paths/bills.rs"),
                ),
                (
                    "users",
                    include_str!("../tests_cfg/duplicated_many_to_many_paths/users.rs"),
                ),
                (
                    "users_saved_bills",
                    include_str!("../tests_cfg/duplicated_many_to_many_paths/users_saved_bills.rs"),
                ),
                (
                    "users_votes",
                    include_str!("../tests_cfg/duplicated_many_to_many_paths/users_votes.rs"),
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
                ("bills", include_str!("../tests_cfg/many_to_many/bills.rs")),
                ("users", include_str!("../tests_cfg/many_to_many/users.rs")),
                (
                    "users_votes",
                    include_str!("../tests_cfg/many_to_many/users_votes.rs"),
                ),
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
                (
                    "bills",
                    include_str!("../tests_cfg/many_to_many_multiple/bills.rs"),
                ),
                (
                    "users",
                    include_str!("../tests_cfg/many_to_many_multiple/users.rs"),
                ),
                (
                    "users_votes",
                    include_str!("../tests_cfg/many_to_many_multiple/users_votes.rs"),
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
                (
                    "bills",
                    include_str!("../tests_cfg/self_referencing/bills.rs"),
                ),
                (
                    "users",
                    include_str!("../tests_cfg/self_referencing/users.rs"),
                ),
            ],
        )
    }

    fn validate_compact_entities(
        table_create_stmts: Vec<TableCreateStatement>,
        files: Vec<(&str, &str)>,
    ) -> Result<(), Box<dyn Error>> {
        let entities: HashMap<_, _> = EntityTransformer::transform(table_create_stmts)?
            .entities
            .into_iter()
            .map(|entity| (entity.table_name.clone(), entity))
            .collect();

        for (entity_name, file_content) in files {
            let entity = entities
                .get(entity_name)
                .expect("Forget to add entity to the list");

            assert_eq!(
                parse_from_file(file_content.as_bytes())?.to_string(),
                EntityWriter::gen_compact_code_blocks(
                    entity,
                    &crate::WithSerde::None,
                    &None,
                    false,
                    false,
                    &Default::default(),
                    &Default::default()
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

    fn parse_from_file<R>(inner: R) -> io::Result<TokenStream>
    where
        R: io::Read,
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
}
