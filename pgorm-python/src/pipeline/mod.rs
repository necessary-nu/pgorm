//! Public pipeline construction through pgorm's existing branded builders.

mod bound;
mod builder;
mod capabilities;
mod construct;
mod expression;
mod recipe;
mod scope;
mod selected;
mod selected_io;
mod source;
mod window;

pub use builder::{PyGrouped, PyPipeline};
pub(crate) use capabilities::operations as capabilities;
pub use expression::PyPipelineExpr;
pub use pgorm_bindings::entity::{SourceBindings, SourceTypes};
pub use selected::{PySelectedSources, PySourceSelection};
pub use source::PySource;

use pyo3::prelude::*;

pub(crate) fn install(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyPipeline>()?;
    module.add_class::<PyGrouped>()?;
    module.add_class::<PyPipelineExpr>()?;
    module.add_class::<scope::PyPipelineBinder>()?;
    module.add_class::<PySource>()?;
    module.add_class::<window::PyOver>()?;
    module.add_class::<PySourceSelection>()?;
    module.add_class::<PySelectedSources>()?;
    module.add_function(wrap_pyfunction!(selected::pipeline_sources, module)?)?;
    module.add_function(wrap_pyfunction!(builder::pipeline_tuples, module)?)?;
    module.add_function(wrap_pyfunction!(source::pipeline_source, module)?)?;
    module.add_function(wrap_pyfunction!(construct::pipeline_literal, module)?)?;
    module.add_function(wrap_pyfunction!(construct::pipeline_alias, module)?)?;
    module.add_function(wrap_pyfunction!(construct::pipeline_col, module)?)?;
    module.add_function(wrap_pyfunction!(construct::pipeline_model_col, module)?)?;
    module.add_function(wrap_pyfunction!(construct::pipeline_role, module)?)?;
    module.add_function(wrap_pyfunction!(construct::pipeline_function, module)?)?;
    module.add_function(wrap_pyfunction!(construct::pipeline_case, module)?)?;
    Ok(())
}
