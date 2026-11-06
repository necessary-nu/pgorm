//! A disjoint model's shape, described from the models Python declares.

use std::sync::Arc;

use pgorm::pgorm_query::{Name, TypeName};
use pgorm::{DisjointColumn, DisjointShape, DisjointTable};
use pyo3::{prelude::*, types::PyDict};
use serde_json::json;

use super::{Codec, Core, NativeDisjoint};
use crate::{
    errors::ConstructionError,
    schema::{DescribedKey, column_of, item, table_name},
    values::PyTypeName,
};

/// One model's table, as Python describes it: its schema, its name, its
/// columns in field order, its key's columns and its unique keys, each column
/// lowered as a model's schema lowers it.
// [spec:pgorm:req:bindings.model-unique-keys+2]
pub(crate) fn table_of(table: &Bound<'_, PyAny>) -> PyResult<DisjointTable> {
    let described = table.cast::<PyDict>()?;
    let columns = item(described, "columns")?
        .extract::<Vec<Bound<'_, PyAny>>>()?
        .iter()
        .map(|column| {
            let (name, def, unstated) = column_of(column)?;
            let column = DisjointColumn::new(name, def);
            Ok(match unstated {
                true => column.with_unstated_default(),
                false => column,
            })
        })
        .collect::<PyResult<Vec<_>>>()?;
    let key: Vec<String> = item(described, "key")?.extract()?;
    let key: Vec<&str> = key.iter().map(String::as_str).collect();
    let unique_keys: Vec<DescribedKey> = item(described, "unique_keys")?.extract()?;
    let construction = |error: pgorm::Error| ConstructionError::new_err(error.to_string());
    let mut table =
        DisjointTable::new(table_name(described)?, columns, &key).map_err(construction)?;
    if item(described, "auto_increment")?.extract::<bool>()? {
        table = table.auto_increment();
    }
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
// [spec:pgorm:req:bindings.disjoint+2]
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
