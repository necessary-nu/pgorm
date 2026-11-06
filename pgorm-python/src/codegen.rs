//! Declared models generated from DDL: pgorm-codegen's Python writer, the
//! one reader and model the Rust entities are generated from, behind the
//! `_models_from_sql` that `pgorm.codegen.models_from_sql` wraps.

use pgorm_codegen::{ModelWriterOptions, TableIdent, UnkindedDisjoint, python_models_from_sql};
use pyo3::prelude::*;

use crate::errors::ConstructionError;

/// A table as `pgorm.codegen` passes it: its schema, if it names one, and
/// its name.
type Table = (Option<String>, String);

fn ident((schema, table): Table) -> TableIdent {
    TableIdent { table, schema }
}

/// The module pgorm-codegen's Python writer writes for `ddl`, whose
/// refusal of the text, or of what it declares, is a `ConstructionError`
/// with pgorm-codegen's message.
// [spec:pgorm:req:bindings.models-from-sql]
#[pyfunction]
#[pyo3(signature = (ddl, schema_name, unkinded_disjoint))]
fn _models_from_sql(
    py: Python<'_>,
    ddl: &str,
    schema_name: Option<String>,
    unkinded_disjoint: Vec<(Table, Vec<Table>, Option<String>)>,
) -> PyResult<String> {
    let options = ModelWriterOptions {
        schema_name,
        unkinded_disjoint: unkinded_disjoint
            .into_iter()
            .map(|(base, kinds, unit)| UnkindedDisjoint {
                base: ident(base),
                kinds: kinds.into_iter().map(ident).collect(),
                unit,
            })
            .collect(),
    };
    let output = py
        .detach(|| python_models_from_sql(ddl, options))
        .map_err(|error| ConstructionError::new_err(error.to_string()))?;
    Ok(output.files.into_iter().map(|file| file.content).collect())
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(_models_from_sql, module)?)
}
