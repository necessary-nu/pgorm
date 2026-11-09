//! pgorm's PRQL-shaped pipeline, from JavaScript.
//!
//! A JavaScript pipeline owns a `pgorm::pipeline::Pipeline`, and every stage
//! returns a new one from a copy. Expressions are recipes (`recipe`) lowered
//! as a stage takes them, inside the binder closure of the stage's `_with`
//! form whenever they bind a value, so that every value enters through
//! pgorm's binder and no placeholder outlives its stage (`scope`).

mod expr;
mod recipe;
mod scope;
mod stage;

use std::sync::Arc;

use neon::prelude::*;
use pgorm::{
    pgorm_query::{Name, TableName, Values},
    pipeline::{self as pl, IntoSource},
};

use super::{Node, args::refuse};
use recipe::Recipe;
use scope::{Scope, lifecycle};

/// Every pipeline export, by module.
pub(super) const EXPORTS: &[&[(&str, super::Build)]] =
    &[expr::EXPORTS, scope::EXPORTS, stage::EXPORTS];

/// A pipeline expression: its recipe, and the scope of the placeholders in
/// it, if it holds any.
#[derive(Debug, Clone)]
pub(crate) struct PlExpr {
    recipe: Arc<Recipe>,
    scope: Option<Arc<Scope>>,
}

/// A relation a pipeline reads.
#[derive(Debug, Clone)]
enum Relation {
    Table(TableName),
    Pipeline(Box<pl::Pipeline>),
    /// A registered entity's table, as its own `IntoSource` names it.
    Entity(crate::entities::EntitySource),
}

/// A relation, and the name it is read under when it has one of its own.
#[derive(Debug, Clone)]
pub(crate) struct Source {
    relation: Relation,
    alias: Option<Name>,
}

impl Source {
    /// A pipeline starting from the relation: a table by its name, its schema
    /// kept, or anything named or embedded as the source it is.
    // [spec:pgorm:req:bindings.pipeline+1]
    fn pipeline(&self) -> pl::Pipeline {
        match (&self.relation, &self.alias) {
            (Relation::Table(TableName::Table(table)), None) => pl::Pipeline::from(table.clone()),
            (Relation::Table(TableName::SchemaTable(schema, table)), None) => {
                pl::Pipeline::from_schema(schema.clone(), table.clone())
            }
            _ => pl::Pipeline::from(self.source()),
        }
    }

    /// The relation as a join's or a set operation's operand. A
    /// schema-qualified table is embedded under its own name, or its alias,
    /// so that its columns stay addressable by one.
    fn source(&self) -> pl::Source {
        let source = match &self.relation {
            Relation::Table(TableName::Table(table)) => table.clone().into_source(),
            Relation::Table(TableName::SchemaTable(schema, table)) => pl::named_runtime(
                pl::Pipeline::from_schema(schema.clone(), table.clone()),
                self.alias.clone().unwrap_or_else(|| table.clone()),
            )
            .into_source(),
            Relation::Pipeline(pipeline) => (**pipeline).clone().into_source(),
            Relation::Entity(entity) => entity.source(),
        };
        match &self.alias {
            Some(alias) => pl::named_runtime(source, alias.clone()).into_source(),
            None => source,
        }
    }
}

/// The pipeline state one JavaScript object owns.
// Each variant is pgorm's own value; the node boxes the lot.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub(crate) enum Part {
    Pipeline(pl::Pipeline),
    /// A pipeline grouped and not yet aggregated, which is no relation.
    Grouped(pl::Grouped),
    Expr(PlExpr),
    Source(Source),
    Over(pl::Over),
    Scope(Arc<Scope>),
}

impl Part {
    /// What the part is, as an error names it.
    pub(crate) fn describe(&self) -> &'static str {
        match self {
            Self::Pipeline(_) => "a Pipeline",
            Self::Grouped(_) => "a grouped pipeline with no aggregate",
            Self::Expr(_) => "a pipeline expression",
            Self::Source(_) => "a pipeline Source",
            Self::Over(_) => "a window",
            Self::Scope(_) => "a binder",
        }
    }
}

impl From<Part> for Node {
    fn from(part: Part) -> Self {
        Node::Pipeline(Box::new(part))
    }
}

impl From<pl::Pipeline> for Node {
    fn from(pipeline: pl::Pipeline) -> Self {
        Part::Pipeline(pipeline).into()
    }
}

/// The SQL and values a pipeline compiles to, through pgorm's
/// `Pipeline::into_sql`; what it judges is a `ConstructionError`.
// [spec:pgorm:req:bindings.pipeline+1]
pub(super) fn compile<'cx>(cx: &mut Cx<'cx>, part: &Part) -> NeonResult<(String, Values)> {
    match part {
        Part::Pipeline(pipeline) => match pipeline.clone().into_sql() {
            Ok(built) => Ok(built),
            Err(error) => refuse(cx, error.to_string()),
        },
        Part::Grouped(_) => refuse(
            cx,
            "a grouped pipeline needs aggregate(..) before it can be inspected or run",
        ),
        other => {
            let what = other.describe();
            cx.throw_type_error(format!("{what} is not a statement to run"))
        }
    }
}

/// An expression made of `operands` and their scopes: refused when two
/// scopes meet, or one has ended, as pgorm's brand refuses it.
// [spec:pgorm:req:bindings.pipeline-binder]
fn compose<'cx>(cx: &mut Cx<'cx>, recipe: Recipe, operands: &[&PlExpr]) -> NeonResult<Node> {
    let mut scope: Option<Arc<Scope>> = None;
    for operand in operands {
        let Some(theirs) = &operand.scope else {
            continue;
        };
        if !theirs.is_open(cx)? {
            return lifecycle(cx, pgorm_bindings::pipeline::PLACEHOLDER_ENDED);
        }
        match &scope {
            Some(ours) if !Arc::ptr_eq(ours, theirs) => {
                return lifecycle(cx, pgorm_bindings::pipeline::TWO_BINDERS);
            }
            _ => scope = Some(theirs.clone()),
        }
    }
    Ok(Part::Expr(PlExpr {
        recipe: Arc::new(recipe),
        scope,
    })
    .into())
}
