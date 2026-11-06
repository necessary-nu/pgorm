//! A disjoint model's shape, described from the models Python declares.

use std::sync::Arc;

use pgorm::{DisjointColumn, DisjointTable};
use pgorm_bindings::entity::{
    DescribedColumn, DescribedKey as SharedKey, DescribedTable, DisjointCore, Generated, Kinded,
};
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyDict};

use super::NativeDisjoint;
use crate::{
    config::DIALECT,
    errors::Raise,
    values::{PyTypeName, Tag, TagPy},
};

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
    let generated = match item(described, "generated")?
        .extract::<Option<String>>()?
        .as_deref()
    {
        Some("always") => Some(Generated::Always),
        Some("by_default") => Some(Generated::ByDefault),
        Some(_) => {
            return Err(PyTypeError::new_err(
                "a column is generated always or by default",
            ));
        }
        None => None,
    };
    DescribedColumn {
        name,
        kind,
        values: values.unwrap_or_default(),
        nullable: item(described, "nullable")?.extract::<bool>()?,
        generated,
        default: item(described, "default")?.extract::<bool>()?,
    }
    .column()
    .py()
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
    let unique_keys: Vec<DescribedKey> = item(described, "unique_keys")?.extract()?;
    DescribedTable {
        schema,
        table: name,
        columns,
        key,
        unique_keys: unique_keys
            .into_iter()
            .map(|(name, columns, nulls_not_distinct)| SharedKey {
                name,
                columns,
                nulls_not_distinct,
            })
            .collect(),
    }
    .table()
    .py()
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
    let mut tables = Vec::with_capacity(kinds.len());
    for (value, table) in kinds {
        tables.push((
            value,
            match table {
                Some((model, table)) => Some((model, table_of(&table)?)),
                None => None,
            },
        ));
    }
    let kinded = kinded.map(|(column, field, kind_type)| Kinded {
        column,
        field,
        kind_type: kind_type.0.clone(),
    });
    let core =
        DisjointCore::model(DIALECT, name, (base_name, table_of(&base)?), kinded, tables).py()?;
    Ok(NativeDisjoint {
        core: Arc::new(core),
    })
}
