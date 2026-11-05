use pgorm::{
    pgorm_query::{IntoNamedTable, NamedTable},
    pipeline::{self as pl, IntoSource},
};
use pyo3::{exceptions::PyTypeError, prelude::*};

use super::{builder::PyPipeline, expression::alias_name};
use crate::{entities::PyEntity, errors::ConstructionError, statements::PyTable};
use pgorm::pgorm_query::Name;

/// The pipeline arm is boxed because it is several times the size of a named
/// table, and a `PySource` is cloned on every composition.
#[derive(Clone, Debug)]
pub(super) enum Relation {
    Table(NamedTable),
    Pipeline(Box<pl::Pipeline>),
}

/// A source descriptor owns its Rust table or pipeline and an optional alias.
#[pyclass(
    name = "PipelineSource",
    module = "pgorm.pipeline",
    frozen,
    from_py_object
)]
#[derive(Clone, Debug)]
pub struct PySource {
    relation: Relation,
    alias: Option<String>,
}

impl PySource {
    pub(super) fn pipeline(&self) -> pl::Pipeline {
        match (&self.relation, &self.alias) {
            (Relation::Table(table), None) => match table.name.schema() {
                Some(schema) => pl::Pipeline::from_schema(
                    Name::runtime(schema.to_string()),
                    Name::runtime(table.name.table().to_string()),
                ),
                None => pl::Pipeline::from(Name::runtime(table.name.table().to_string())),
            },
            _ => pl::Pipeline::from(self.source()),
        }
    }

    pub(super) fn source(&self) -> pl::Source {
        let source = match &self.relation {
            Relation::Table(table) => match table.name.schema() {
                Some(schema) => {
                    let table_name = table.name.table().to_string();
                    let pipeline = pl::Pipeline::from_schema(
                        Name::runtime(schema.to_string()),
                        Name::runtime(&table_name),
                    );
                    pl::named_runtime(
                        pipeline,
                        Name::runtime(self.alias.clone().unwrap_or(table_name)),
                    )
                    .into_source()
                }
                None => Name::runtime(table.name.table().to_string()).into_source(),
            },
            Relation::Pipeline(pipeline) => (**pipeline).clone().into_source(),
        };
        match &self.alias {
            Some(name) => pl::named_runtime(source, Name::runtime(name)).into_source(),
            None => source,
        }
    }
}

#[pymethods]
impl PySource {
    fn named(&self, name: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            relation: self.relation.clone(),
            alias: Some(alias_name(name)?),
        })
    }

    // [spec:pgorm:req:python.truth-testing+4]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "pipeline sources cannot be tested as Python booleans",
        ))
    }
}

#[pyfunction]
pub(super) fn pipeline_source(value: &Bound<'_, PyAny>) -> PyResult<PySource> {
    if let Ok(source) = value.extract::<PyRef<'_, PySource>>() {
        return Ok(source.clone());
    }
    if let Ok(pipeline) = value.extract::<PyRef<'_, PyPipeline>>() {
        return Ok(PySource {
            relation: Relation::Pipeline(Box::new(pipeline.inner.clone())),
            alias: None,
        });
    }
    if let Ok(table) = value.extract::<PyRef<'_, PyTable>>() {
        return Ok(table_source(&table));
    }
    if let Ok(entity) = value.extract::<PyRef<'_, PyEntity>>() {
        let info = entity.backend.info();
        let table = match &info.schema {
            Some(schema) => (Name::runtime(schema), Name::runtime(&info.table)).into_named_table(),
            None => Name::runtime(&info.table).into_named_table(),
        };
        return Ok(PySource {
            relation: Relation::Table(table),
            alias: None,
        });
    }
    // A module that embeds the binding without its Python package, as an
    // application's Rust tests do, has no declared models to be given.
    // [spec:pgorm:req:bindings.pipeline-models]
    if let Ok(model) = value
        .py()
        .import("pgorm.models")
        .and_then(|models| models.getattr("Model"))
        && value.is_instance(&model)?
    {
        let table = value.getattr("table")?;
        let table = table.extract::<PyRef<'_, PyTable>>()?;
        return Ok(table_source(&table));
    }
    Err(PyTypeError::new_err(
        "pipeline source requires a Table, Model, Entity, Pipeline or named Source",
    ))
}

/// A table read under its alias when it has one: a `Table`'s, or a declared
/// model's.
fn table_source(table: &PyTable) -> PySource {
    PySource {
        relation: Relation::Table(table.inner.clone()),
        alias: table.inner.alias.as_ref().map(|alias| alias.to_string()),
    }
}
