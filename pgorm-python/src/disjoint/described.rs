//! A disjoint model's shape, described from the models Python declares.

use std::sync::Arc;

use pgorm::pgorm_query::{ColumnType, Name, TableName, TypeName};
use pgorm::{ColumnTypeTrait, DisjointColumn, DisjointShape, DisjointTable};
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyDict};
use serde_json::json;

use super::{Codec, Core, NativeDisjoint};
use crate::{
    errors::ConstructionError,
    values::{PyTypeName, Tag},
};

/// The column type a model column of `tag` is written with: a scalar kind's
/// own (`ArrayType::column_type`), an enum of the labels `values` lists, a
/// created range over its subtype's, and an array of its element's.
fn column_type(tag: &Tag, values: Option<&[String]>) -> PyResult<ColumnType> {
    Ok(match tag {
        Tag::Scalar(kind) => kind.column_type().ok_or_else(|| {
            ConstructionError::new_err(format!(
                "no column decodes as `{}`, so a disjoint model's column cannot be one",
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
            let subtype = Arc::new(column_type(&Tag::Scalar(kind.subtype.clone()), None)?);
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
        Tag::Array(element) => ColumnType::Array(Arc::new(column_type(element, values)?)),
    })
}

fn item<'py>(table: &Bound<'py, PyDict>, key: &str) -> PyResult<Bound<'py, PyAny>> {
    table
        .get_item(key)?
        .ok_or_else(|| PyTypeError::new_err(format!("a described table has a `{key}`")))
}

/// One model column, as Python describes it.
fn column_of(column: &Bound<'_, PyAny>) -> PyResult<DisjointColumn> {
    let described = column.cast::<PyDict>()?;
    let name: String = item(described, "name")?.extract()?;
    let kind = Tag::parse(&item(described, "kind")?)?;
    let kind = match item(described, "array")?.extract::<bool>()? {
        true => Tag::Array(Box::new(kind)),
        false => kind,
    };
    let values: Option<Vec<String>> = item(described, "values")?.extract()?;
    let mut def = column_type(&kind, values.as_deref())?.def();
    if item(described, "nullable")?.extract::<bool>()? {
        def = def.nullable();
    }
    def = match item(described, "generated")?
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
    };
    let column = DisjointColumn::new(name, def);
    Ok(match item(described, "default")?.extract::<bool>()? {
        true => column.with_unstated_default(),
        false => column,
    })
}

/// A unique key of a model's table, as Python describes it: the
/// constraint's name or `None`, its columns' names in key order, and whether
/// nulls are equal under it.
type DescribedKey = (Option<String>, Vec<String>, bool);

/// One model's table, as Python describes it: its schema, its name, its
/// columns in field order, its key's columns and its unique keys.
// [spec:pgorm:req:bindings.model-unique-keys+2]
pub(crate) fn table_of(table: &Bound<'_, PyAny>) -> PyResult<DisjointTable> {
    let described = table.cast::<PyDict>()?;
    let schema: Option<String> = item(described, "schema")?.extract()?;
    let name: String = item(described, "table")?.extract()?;
    let columns = item(described, "columns")?
        .extract::<Vec<Bound<'_, PyAny>>>()?
        .iter()
        .map(column_of)
        .collect::<PyResult<Vec<_>>>()?;
    let key: Vec<String> = item(described, "key")?.extract()?;
    let table = match schema {
        Some(schema) => TableName::SchemaTable(Name::runtime(schema), Name::runtime(name)),
        None => TableName::Table(Name::runtime(name)),
    };
    let key: Vec<&str> = key.iter().map(String::as_str).collect();
    let unique_keys: Vec<DescribedKey> = item(described, "unique_keys")?.extract()?;
    let construction = |error: pgorm::Error| ConstructionError::new_err(error.to_string());
    let mut table = DisjointTable::new(table, columns, &key).map_err(construction)?;
    for (name, columns, nulls_not_distinct) in &unique_keys {
        let columns: Vec<&str> = columns.iter().map(String::as_str).collect();
        table = table
            .with_unique_key(name.as_deref(), &columns, *nulls_not_distinct)
            .map_err(construction)?;
    }
    Ok(table)
}

/// A model's name beside the description of its table.
type Described<'py> = (String, Bound<'py, PyAny>);

/// Where a kinded model's base holds each row's kind: the column, the field
/// it is the column of, and the kind type.
type KindedBy<'py> = (String, String, PyRef<'py, PyTypeName>);

/// `_disjoint_model(name, base, kinded, kinds)`: the disjoint model `name`
/// over the table `base` describes and each kind in order, its value beside
/// its model's name and table, or `None` for a kind without one. A kinded
/// model's `kinded` is its kind column, the field that column is of and its
/// kind type; an unkinded model's is `None`, its base holding no kind. What
/// the shape refuses is a `ConstructionError`, raised as the model is
/// declared.
// [spec:pgorm:req:bindings.disjoint+1]
// [spec:pgorm:req:bindings.disjoint-unkinded]
#[pyfunction(name = "_disjoint_model")]
pub(crate) fn disjoint_model(
    name: &str,
    base: Described<'_>,
    kinded: Option<KindedBy<'_>>,
    kinds: Vec<(String, Option<Described<'_>>)>,
) -> PyResult<NativeDisjoint> {
    let (base_name, base) = base;
    let mut described = Vec::with_capacity(kinds.len());
    let mut tables = Vec::with_capacity(kinds.len());
    for (value, table) in kinds {
        match table {
            Some((model, table)) => {
                described.push(json!({"value": value, "model": model}));
                tables.push((value, Some(table_of(&table)?)));
            }
            None => {
                described.push(json!({"value": value, "model": null}));
                tables.push((value, None));
            }
        }
    }
    let base = table_of(&base)?;
    let (shape, kind_field, kind_type) = match &kinded {
        Some((kind_column, kind_field, kind_type)) => {
            let mut type_name = TypeName::new(Name::runtime(&kind_type.name));
            if let Some(schema) = &kind_type.schema {
                type_name = type_name.schema(Name::runtime(schema));
            }
            (
                DisjointShape::new(base, kind_column, type_name, tables),
                Some(kind_field.as_str()),
                json!({"schema": kind_type.schema, "name": kind_type.name}),
            )
        }
        None => (
            DisjointShape::unkinded(base, tables),
            None,
            json!({"schema": null, "name": null}),
        ),
    };
    let shape = shape.map_err(|error| ConstructionError::new_err(error.to_string()))?;
    Ok(NativeDisjoint {
        core: Arc::new(Core {
            shape,
            codec: Codec::Models,
            description: json!({
                "name": name,
                "base": base_name,
                "kind_field": kind_field,
                "kind_type": kind_type,
                "kinds": described,
            }),
            rust: None,
        }),
    })
}
