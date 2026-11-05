//! A declared model's table as pgorm's `TableShape`, which its schema and a
//! disjoint model's tables are written from.

use std::sync::Arc;

use pgorm::pgorm_query::{ColumnType, GeneratedKind, Name, TableName};
use pgorm::{
    CaseStyle, ColumnDef, ColumnPairs, ColumnTypeTrait, RelationDef, Schema, TableColumn,
    TableShape,
};
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyDict};

use super::{
    PyDataType, PyEntitySchema,
    args::{ACTIONS, choice},
};
use crate::{errors::ConstructionError, expressions, values::Tag};

/// The column type a model column of `tag` is written with: its
/// `column_type` where it declares one, made an array where the column is;
/// otherwise a scalar kind's own (`ArrayType::column_type`), an enum of the
/// labels `values` lists, a created range over its subtype's, and an array of
/// its element's.
// [spec:pgorm:req:bindings.schema-from-model]
fn column_type(
    tag: &Tag,
    values: Option<&[String]>,
    declared: Option<&ColumnType>,
) -> PyResult<ColumnType> {
    if let Some(declared) = declared {
        return Ok(match tag {
            Tag::Array(_) => ColumnType::Array(Arc::new(declared.clone())),
            _ => declared.clone(),
        });
    }
    Ok(match tag {
        Tag::Scalar(kind) => kind.column_type().ok_or_else(|| {
            ConstructionError::new_err(format!(
                "no column decodes as `{}`, so a model's column cannot be one",
                kind.kind_name()
            ))
        })?,
        Tag::Enum(name) => ColumnType::Enum {
            name: Name::runtime(&name.name),
            schema: name.schema.as_ref().map(Name::runtime),
            variants: values
                .unwrap_or_default()
                .iter()
                .map(Name::runtime)
                .collect(),
        },
        Tag::Created(kind) => {
            let subtype = Arc::new(column_type(&Tag::Scalar(kind.subtype.clone()), None, None)?);
            let (name, schema) = (
                Name::runtime(&kind.name.name),
                kind.name.schema.as_ref().map(Name::runtime),
            );
            match kind.multirange {
                true => ColumnType::CreatedMultirange {
                    name,
                    schema,
                    subtype,
                },
                false => ColumnType::CreatedRange {
                    name,
                    schema,
                    subtype,
                },
            }
        }
        Tag::Array(element) => ColumnType::Array(Arc::new(column_type(element, values, None)?)),
    })
}

pub(crate) fn item<'py>(table: &Bound<'py, PyDict>, key: &str) -> PyResult<Bound<'py, PyAny>> {
    table
        .get_item(key)?
        .ok_or_else(|| PyTypeError::new_err(format!("a described table has a `{key}`")))
}

/// The type a column declares with `column_type`: any type a `ColumnDef`
/// takes but an array, since an array column's is its elements'.
// [spec:pgorm:req:bindings.schema-from-model]
fn declared_type(value: &Bound<'_, PyAny>) -> PyResult<Option<ColumnType>> {
    if value.is_none() {
        return Ok(None);
    }
    let declared = PyDataType::coerce(value)?.inner;
    if matches!(declared, ColumnType::Array(_)) {
        return Err(ConstructionError::new_err(
            "a column's column_type is the type of its values, an array column's its \
             elements': declare the array with array=True",
        ));
    }
    Ok(Some(declared))
}

/// What fills a column an insert leaves out, as `def` writes it: a stated
/// default, an identity or a generation expression.
// [spec:pgorm:req:bindings.schema-from-model]
fn filled(described: &Bound<'_, PyDict>, mut def: ColumnDef) -> PyResult<ColumnDef> {
    let default_value = item(described, "default_value")?;
    if !default_value.is_none() {
        def = def.default(expressions::coerce(&default_value)?.inner);
    }
    let default_expr = item(described, "default_expr")?;
    if !default_expr.is_none() {
        def = def.default(expressions::require_expr(&default_expr)?.inner);
    }
    for (key, kind) in [
        ("generated_stored", GeneratedKind::Stored),
        ("generated_virtual", GeneratedKind::Virtual),
    ] {
        let expression = item(described, key)?;
        if !expression.is_none() {
            return Ok(def.generated(expressions::require_expr(&expression)?.inner, kind));
        }
    }
    Ok(
        match item(described, "generated")?
            .extract::<Option<String>>()?
            .as_deref()
        {
            Some("always") => def.identity(),
            Some("by_default") => def.identity_by_default(),
            Some(_) => {
                return Err(PyTypeError::new_err(
                    "a column is generated always or by default",
                ));
            }
            None => def,
        },
    )
}

/// One model column, as Python describes it: its name, its definition, and
/// whether the database fills it by a default the declaration does not
/// state.
// [spec:pgorm:req:bindings.schema-from-model]
pub(crate) fn column_of(column: &Bound<'_, PyAny>) -> PyResult<(String, ColumnDef, bool)> {
    let described = column.cast::<PyDict>()?;
    let name: String = item(described, "name")?.extract()?;
    let kind = Tag::parse(&item(described, "kind")?)?;
    let kind = match item(described, "array")?.extract::<bool>()? {
        true => Tag::Array(Box::new(kind)),
        false => kind,
    };
    let values: Option<Vec<String>> = item(described, "values")?.extract()?;
    let declared = declared_type(&item(described, "column_type")?)?;
    let mut def = column_type(&kind, values.as_deref(), declared.as_ref())?.def();
    if item(described, "nullable")?.extract::<bool>()? {
        def = def.nullable();
    }
    if item(described, "indexed")?.extract::<bool>()? {
        def = def.indexed();
    }
    if let Some(comment) = item(described, "comment")?.extract::<Option<String>>()? {
        def = def.comment(&comment);
    }
    let def = filled(described, def)?;
    let unstated = item(described, "default")?.extract::<bool>()?;
    Ok((name, def, unstated))
}

/// A unique key of a model's table, as Python describes it: the
/// constraint's name or `None`, its columns' names in key order, and whether
/// nulls are equal under it.
pub(crate) type DescribedKey = (Option<String>, Vec<String>, bool);

/// The table a described model names, schema-qualified where it is.
pub(crate) fn table_name(described: &Bound<'_, PyDict>) -> PyResult<TableName> {
    let schema: Option<String> = item(described, "schema")?.extract()?;
    let name: String = item(described, "table")?.extract()?;
    Ok(match schema {
        Some(schema) => TableName::SchemaTable(Name::runtime(schema), Name::runtime(name)),
        None => TableName::Table(Name::runtime(name)),
    })
}

/// One model's table, as Python describes it: its schema, its name, its
/// columns in field order, its key's columns and how the key is generated,
/// its unique keys and its comment.
// [spec:pgorm:req:bindings.schema-from-model]
// [spec:pgorm:req:bindings.model-unique-keys+2]
pub(crate) fn shape_of(table: &Bound<'_, PyAny>) -> PyResult<TableShape> {
    let refused = |error: pgorm::Error| ConstructionError::new_err(error.to_string());
    let described = table.cast::<PyDict>()?;
    let columns = item(described, "columns")?
        .extract::<Vec<Bound<'_, PyAny>>>()?
        .iter()
        .map(|column| {
            let (name, def, unstated) = column_of(column)?;
            let column = TableColumn::new(name, def);
            Ok(match unstated {
                true => column.with_unstated_default(),
                false => column,
            })
        })
        .collect::<PyResult<Vec<_>>>()?;
    let mut shape = TableShape::new(table_name(described)?, columns).map_err(refused)?;
    let key: Vec<String> = item(described, "key")?.extract()?;
    if !key.is_empty() {
        let key: Vec<&str> = key.iter().map(String::as_str).collect();
        shape = shape.with_primary_key(&key).map_err(refused)?;
    }
    if item(described, "auto_increment")?.extract::<bool>()? {
        shape = shape.auto_increment();
    }
    if item(described, "without_overlaps")?.extract::<bool>()? {
        shape = shape.without_overlaps();
    }
    let unique_keys: Vec<DescribedKey> = item(described, "unique_keys")?.extract()?;
    for (name, columns, nulls_not_distinct) in &unique_keys {
        let columns: Vec<&str> = columns.iter().map(String::as_str).collect();
        shape = shape
            .with_unique_key(name.as_deref(), &columns, *nulls_not_distinct)
            .map_err(refused)?;
    }
    if let Some(comment) = item(described, "comment")?.extract::<Option<String>>()? {
        shape = shape.with_comment(comment);
    }
    Ok(shape)
}

/// `shape` with the foreign key of one `belongs_to` relation, as Python
/// describes it: the table it reaches, the columns it pairs at each end, its
/// actions and its constraint's name.
// [spec:pgorm:req:bindings.schema-from-model]
fn with_relation(shape: TableShape, relation: &Bound<'_, PyAny>) -> PyResult<TableShape> {
    let refused = |error: pgorm::Error| ConstructionError::new_err(error.to_string());
    let described = relation.cast::<PyDict>()?;
    let from: Vec<String> = item(described, "from")?.extract()?;
    let to: Vec<String> = item(described, "to")?.extract()?;
    let target = table_name(described)?;
    let (Some(first_from), Some(first_to)) = (from.first(), to.first()) else {
        return Err(ConstructionError::new_err(
            "a relation pairs at least one column",
        ));
    };
    let mut pairs = ColumnPairs::new(Name::runtime(first_from), Name::runtime(first_to));
    for (from, to) in from.iter().zip(&to).skip(1) {
        pairs.push(Name::runtime(from), Name::runtime(to));
    }
    let label = format!("{}({})", target.table().to_string(), from.join(", "));
    let mut def = RelationDef::belongs_to(shape.table().clone(), target, pairs);
    let on_delete: Option<String> = item(described, "on_delete")?.extract()?;
    def.on_delete = choice(on_delete.as_deref(), "an ON DELETE action", ACTIONS)?;
    let on_update: Option<String> = item(described, "on_update")?.extract()?;
    def.on_update = choice(on_update.as_deref(), "an ON UPDATE action", ACTIONS)?;
    def.fk_name = item(described, "fk_name")?.extract()?;
    shape.with_relation(label, def).map_err(refused)
}

/// `_model_table(table)`: the table a model declares, checked as pgorm's
/// `TableShape` checks it, so what it cannot describe is refused as the
/// model is declared.
// [spec:pgorm:req:bindings.schema-from-model]
#[pyfunction(name = "_model_table")]
pub(super) fn model_table(table: &Bound<'_, PyAny>) -> PyResult<()> {
    shape_of(table).map(drop)
}

/// `_model_schema(table, relations)`: the schema of the table a model
/// declares, with a foreign key for each `belongs_to` relation in
/// `relations`, as pgorm's `Schema` writes a `TableShape`'s.
// [spec:pgorm:req:bindings.schema-from-model]
#[pyfunction(name = "_model_schema")]
pub(super) fn model_schema(
    table: &Bound<'_, PyAny>,
    relations: Vec<Bound<'_, PyAny>>,
) -> PyResult<PyEntitySchema> {
    let refused = |error: pgorm::Error| ConstructionError::new_err(error.to_string());
    let mut shape = shape_of(table)?;
    for relation in &relations {
        shape = with_relation(shape, relation)?;
    }
    let schema = Schema::new();
    Ok(PyEntitySchema {
        table: schema.create_table_from_shape(&shape).map_err(refused)?,
        enums: schema.create_enum_from_shape(&shape),
        indexes: schema.create_index_from_shape(&shape),
        comments: schema.create_comments_from_shape(&shape),
    })
}

/// `_renamed(field, style)`: the column a model's `rename_all` names the
/// field `field`, as the entity derive's `rename_all` names a field's.
// [spec:pgorm:req:bindings.schema-from-model]
#[pyfunction(name = "_renamed")]
pub(super) fn renamed(field: &str, style: &str) -> PyResult<String> {
    let style: CaseStyle = style
        .parse()
        .map_err(|error: pgorm::UnknownCaseStyle| PyTypeError::new_err(error.to_string()))?;
    Ok(style.convert(field))
}
