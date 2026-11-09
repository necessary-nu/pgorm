//! A pipeline's sources and stages. A stage whose expressions bind nothing
//! takes them as pgorm's plain transform does; one that binds — a value an
//! operand was given, or a `With` function's placeholders — takes them through
//! the transform's `_with` form, lowering them inside the binder closure.

use std::sync::Arc;

use neon::prelude::*;
use pgorm::{
    pgorm_query::{TableName, TypeName},
    pipeline as pl,
};

use super::{
    super::{
        Node,
        args::{absent, arg, items, node, refuse, this},
    },
    Part, PlExpr, Relation, Source,
    expr::operand,
    scope::{Scope, lifecycle, scope_at},
};
use crate::{errors::throw, models::CastsHandle};
use pgorm_bindings::pipeline::{self as stages, Plan, Stage};

pub(super) const EXPORTS: &[(&str, super::super::Build)] = &[
    ("pipelineSource", source),
    ("pipelineNamed", named),
    ("pipelineFrom", from),
    ("pipelineFilter", filter),
    ("pipelineList", list),
    ("pipelineGroup", group),
    ("pipelineAggregate", aggregate),
    ("pipelineWindow", window),
    ("pipelineTake", take),
    ("pipelineJoin", join),
    ("pipelineSet", set),
    ("pipelineDistinct", distinct),
    ("pipelineSelectDescribed", select_described),
    ("pipelineReadColumns", read_columns),
    ("pipelineOver", over),
    ("pipelineOverKeys", over_keys),
    ("pipelineOverFrame", over_frame),
];

/// The plan for `expressions`, taken by a stage whose `With` function had
/// `scope`, or by a plain stage when it is `None`: a placeholder from any
/// other scope is refused, as pgorm's brand refuses it.
// [spec:pgorm:req:bindings.pipeline-binder]
fn plan<'cx>(
    cx: &mut Cx<'cx>,
    expressions: Vec<PlExpr>,
    scope: Option<Arc<Scope>>,
) -> NeonResult<Plan> {
    for expression in &expressions {
        let Some(theirs) = &expression.scope else {
            continue;
        };
        let ours = scope.as_ref().is_some_and(|ours| Arc::ptr_eq(ours, theirs));
        if !ours {
            return lifecycle(
                cx,
                "a placeholder belongs to the stage its binder's function returns it to: not a \
                 plain stage, a window, another function's stage or another pipeline's",
            );
        }
    }
    let values = match &scope {
        Some(scope) => scope.take(cx)?,
        None => Vec::new(),
    };
    Ok(Plan {
        recipes: expressions
            .into_iter()
            .map(|expression| expression.recipe)
            .collect(),
        values,
    })
}

fn expressions_at<'cx>(cx: &mut FunctionContext<'cx>, index: usize) -> NeonResult<Vec<PlExpr>> {
    let value = arg(cx, index);
    let values = items(cx, value)?;
    values.into_iter().map(|value| operand(cx, value)).collect()
}

/// The scope at `index`, or `None` for a plain stage.
fn optional_scope<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
) -> NeonResult<Option<Arc<Scope>>> {
    let value = arg(cx, index);
    if absent(cx, value) {
        Ok(None)
    } else {
        scope_at(cx, index).map(Some)
    }
}

/// The plan of the expressions at `index` and the scope after them.
fn plan_at<'cx>(cx: &mut FunctionContext<'cx>, index: usize) -> NeonResult<Plan> {
    let expressions = expressions_at(cx, index)?;
    let scope = optional_scope(cx, index + 1)?;
    plan(cx, expressions, scope)
}

fn pipeline_at<'cx>(cx: &mut FunctionContext<'cx>, index: usize) -> NeonResult<pl::Pipeline> {
    let value = arg(cx, index);
    match node(cx, value) {
        Some(Node::Pipeline(part)) => match *part {
            Part::Pipeline(pipeline) => Ok(pipeline),
            other => cx.throw_type_error(format!("expected a Pipeline, got {}", other.describe())),
        },
        _ => cx.throw_type_error("expected a Pipeline"),
    }
}

/// A relation: a `Table`, read under its alias when it has one, a table's
/// name, a `Pipeline` embedded whole, a `Source`, or a registered entity's
/// table, its schema from the registration.
// [spec:pgorm:req:bindings.pipeline+1]
// [spec:pgorm:req:bindings.pipeline-sources+3]
fn relation<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<Source> {
    if let Some(entity) = crate::entities::entity_source(cx, value) {
        return Ok(Source {
            relation: Relation::Entity(entity),
            alias: None,
        });
    }
    // [spec:pgorm:req:napi.pipeline-table-names]
    if value.is_a::<JsString, _>(cx) {
        let name = super::super::args::name(cx, value)?;
        return Ok(Source {
            relation: Relation::Table(TableName::Table(name)),
            alias: None,
        });
    }
    match node(cx, value) {
        Some(Node::Table(table)) => Ok(Source {
            relation: Relation::Table(table.name),
            alias: table.alias,
        }),
        Some(Node::Pipeline(part)) => match *part {
            Part::Pipeline(pipeline) => Ok(Source {
                relation: Relation::Pipeline(Box::new(pipeline)),
                alias: None,
            }),
            Part::Source(source) => Ok(source),
            other => cx.throw_type_error(format!(
                "a pipeline reads a Table, a Pipeline or a Source, not {}",
                other.describe()
            )),
        },
        _ => {
            cx.throw_type_error("a pipeline reads a Table, a table's name, a Pipeline or a Source")
        }
    }
}

fn relation_at<'cx>(cx: &mut FunctionContext<'cx>, index: usize) -> NeonResult<Source> {
    let value = arg(cx, index);
    relation(cx, value)
}

fn source(cx: &mut FunctionContext) -> NeonResult<Node> {
    let source = relation_at(cx, 0)?;
    Ok(Part::Source(source).into())
}

/// `pipelineNamed(source, name)`: the relation read under a name of its own,
/// which replaces any it had.
// [spec:pgorm:req:bindings.pipeline+1]
fn named(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut source = relation_at(cx, 0)?;
    let name = super::super::args::name_at(cx, 1)?;
    source.alias = Some(name);
    Ok(Part::Source(source).into())
}

/// `pipelineFrom(source)`.
// [spec:pgorm:req:bindings.pipeline+1]
fn from(cx: &mut FunctionContext) -> NeonResult<Node> {
    let source = relation_at(cx, 0)?;
    Ok(source.pipeline().into())
}

/// `pipelineFilter(pipeline, predicate, scope)`.
// [spec:pgorm:req:bindings.pipeline+1]
fn filter(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pipeline = pipeline_at(cx, 0)?;
    let predicate = arg(cx, 1);
    let predicate = operand(cx, predicate)?;
    let scope = optional_scope(cx, 2)?;
    let plan = plan(cx, vec![predicate], scope)?;
    stages::filter(pipeline, plan)
        .or_else(|failure| throw(cx, failure))
        .map(Into::into)
}

/// A list stage: `stage` over the expressions at `index` and the scope after
/// them, plainly when they bind nothing.
fn list_stage(
    cx: &mut FunctionContext,
    pipeline: pl::Pipeline,
    stage: Stage,
    index: usize,
) -> NeonResult<Node> {
    let plan = plan_at(cx, index)?;
    stages::list(pipeline, stage, plan)
        .or_else(|failure| throw(cx, failure))
        .map(Into::into)
}

/// `pipelineList(pipeline, "derive" | "select" | "sort", expressions, scope)`.
// [spec:pgorm:req:bindings.pipeline+1]
fn list(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pipeline = pipeline_at(cx, 0)?;
    let stage = super::super::args::choice(cx, 1, "a stage", &["derive", "select", "sort"])?;
    let stage = match stage {
        "derive" => Stage::Derive,
        "select" => Stage::Select,
        _ => Stage::Sort,
    };
    list_stage(cx, pipeline, stage, 2)
}

/// `pipelineWindow(pipeline, over, expressions, scope)`.
fn window(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pipeline = pipeline_at(cx, 0)?;
    let over = over_at(cx, 1)?;
    list_stage(cx, pipeline, Stage::Window(over), 2)
}

/// `pipelineGroup(pipeline, keys, scope)`: a grouping, no relation until it
/// is aggregated.
// [spec:pgorm:req:bindings.pipeline+1]
fn group(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pipeline = pipeline_at(cx, 0)?;
    let plan = plan_at(cx, 1)?;
    let grouped = stages::group(pipeline, plan).or_else(|failure| throw(cx, failure))?;
    Ok(Part::Grouped(grouped).into())
}

/// `pipelineAggregate(grouped, aggregates, scope)`.
fn aggregate(cx: &mut FunctionContext) -> NeonResult<Node> {
    let grouped = match this(cx, 0)? {
        Node::Pipeline(part) => match *part {
            Part::Grouped(grouped) => grouped,
            other => {
                return cx.throw_type_error(format!(
                    "expected a grouped pipeline, got {}",
                    other.describe()
                ));
            }
        },
        other => {
            return cx.throw_type_error(format!(
                "expected a grouped pipeline, got {}",
                other.describe()
            ));
        }
    };
    let plan = plan_at(cx, 1)?;
    stages::aggregate(grouped, plan)
        .or_else(|failure| throw(cx, failure))
        .map(Into::into)
}

/// A row count for `take`: an integer number or `bigint` within `bigint`.
fn rows<'cx>(cx: &mut FunctionContext<'cx>, index: usize) -> NeonResult<i64> {
    let value = arg(cx, index);
    super::super::schema::integer(cx, value, "a row count")
}

/// `pipelineTake(pipeline, count)` or `pipelineTake(pipeline, start, end)`,
/// the inclusive 1-based range PRQL takes. A count is no expression: PRQL
/// refuses a bound `LIMIT`.
// [spec:pgorm:req:bindings.pipeline+1]
fn take(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pipeline = pipeline_at(cx, 0)?;
    let start = rows(cx, 1)?;
    let end = arg(cx, 2);
    if absent(cx, end) {
        return Ok(pipeline.take(start).into());
    }
    let end = rows(cx, 2)?;
    Ok(pipeline.take_range(start..=end).into())
}

/// `pipelineJoin(pipeline, kind, source, on, scope)`.
// [spec:pgorm:req:bindings.pipeline+1]
fn join(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pipeline = pipeline_at(cx, 0)?;
    let side =
        super::super::args::choice(cx, 1, "a join's kind", &["inner", "left", "right", "full"])?;
    let side = match side {
        "inner" => pl::JoinSide::Inner,
        "left" => pl::JoinSide::Left,
        "right" => pl::JoinSide::Right,
        _ => pl::JoinSide::Full,
    };
    let joined = relation_at(cx, 2)?.source();
    let on = arg(cx, 3);
    let on = operand(cx, on)?;
    let scope = optional_scope(cx, 4)?;
    let plan = plan(cx, vec![on], scope)?;
    stages::join(pipeline, side, joined, plan)
        .or_else(|failure| throw(cx, failure))
        .map(Into::into)
}

/// `pipelineSet(pipeline, "append" | "intersect" | "remove", source)`.
fn set(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pipeline = pipeline_at(cx, 0)?;
    let operation =
        super::super::args::choice(cx, 1, "a set operation", &["append", "intersect", "remove"])?;
    let other = relation_at(cx, 2)?.source();
    Ok((match operation {
        "append" => pipeline.append(other),
        "intersect" => pipeline.intersect(other),
        _ => pipeline.remove(other),
    })
    .into())
}

fn distinct(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pipeline = pipeline_at(cx, 0)?;
    Ok(pipeline.distinct().into())
}

/// What a pipeline reads a declared model's column as, from `[name, readAs,
/// array]`: `readAs` the column's casts, whose `read_cast` it is, a type read
/// as an array of it with `array`, or null, the column read as it stands.
// [spec:pgorm:req:bindings.pipeline-models+1]
fn read_column<'cx>(
    cx: &mut FunctionContext<'cx>,
    column: Handle<'cx, JsValue>,
) -> NeonResult<(pgorm::pgorm_query::Name, Option<TypeName>)> {
    let column = items(cx, column)?;
    let (Some(name), Some(read_as), Some(array)) = (column.first(), column.get(1), column.get(2))
    else {
        return cx.throw_type_error("a described column is [name, readAs, array]");
    };
    let name = super::super::args::name(cx, *name)?;
    let array = array.downcast_or_throw::<JsBoolean, _>(cx)?.value(cx);
    if absent(cx, *read_as) {
        return Ok((name, None));
    }
    if let Ok(casts) = read_as.downcast::<JsBox<CastsHandle>, _>(cx) {
        return Ok((name, casts.0.read_cast()));
    }
    let type_name = super::super::args::type_name(cx, *read_as)?;
    Ok((
        name,
        Some(match array {
            true => type_name.array(),
            false => type_name,
        }),
    ))
}

/// `pipelineSelectDescribed(pipeline, sources)`: pgorm's `select_described`,
/// each source `[qualifier, columns]` and each column as `read_column` reads
/// it.
// [spec:pgorm:req:bindings.pipeline-models+1]
fn select_described(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pipeline = pipeline_at(cx, 0)?;
    let listed = arg(cx, 1);
    let mut sources = Vec::new();
    for source in items(cx, listed)? {
        let source = items(cx, source)?;
        let (Some(qualifier), Some(columns)) = (source.first(), source.get(1)) else {
            return cx.throw_type_error("a described source is [qualifier, columns]");
        };
        let qualifier = super::super::args::name(cx, *qualifier)?;
        let mut described = pl::DescribedSource::new(qualifier);
        for column in items(cx, *columns)? {
            let (name, read_as) = read_column(cx, column)?;
            described = described.column(name, read_as);
        }
        sources.push(described);
    }
    match pipeline.select_described(sources) {
        Ok(pipeline) => Ok(pipeline.into()),
        Err(error) => refuse(cx, error.to_string()),
    }
}

/// `pipelineReadColumns(pipeline, columns)`: pgorm's `read_columns`, the
/// pipeline compiled as a declared model's record reads it, each column as
/// `read_column` reads it.
// [spec:pgorm:req:bindings.pipeline-models+1]
fn read_columns(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pipeline = pipeline_at(cx, 0)?;
    let listed = arg(cx, 1);
    let mut columns = Vec::new();
    for column in items(cx, listed)? {
        let (name, read_as) = read_column(cx, column)?;
        columns.push((pgorm::pgorm_query::SqlName::to_string(&*name), read_as));
    }
    Ok(pipeline.read_columns(columns).into())
}

fn over_at<'cx>(cx: &mut FunctionContext<'cx>, index: usize) -> NeonResult<pl::Over> {
    let value = arg(cx, index);
    match node(cx, value) {
        Some(Node::Pipeline(part)) => match *part {
            Part::Over(over) => Ok(over),
            other => cx.throw_type_error(format!("expected a window, got {}", other.describe())),
        },
        _ => cx.throw_type_error("expected a window from over()"),
    }
}

fn over(_: &mut FunctionContext) -> NeonResult<Node> {
    Ok(Part::Over(pl::over()).into())
}

/// `pipelineOverKeys(over, "by" | "sortBy", keys)`: a window's partition or
/// ordering, which take no value: pgorm's `Over` takes only expressions that
/// bind nothing.
// [spec:pgorm:req:bindings.pipeline-binder]
fn over_keys(cx: &mut FunctionContext) -> NeonResult<Node> {
    let over = over_at(cx, 0)?;
    let which = super::super::args::choice(cx, 1, "a window's keys", &["by", "sortBy"])?;
    let keys = expressions_at(cx, 2)?;
    let plan = plan(cx, keys, None)?;
    if plan.binds() {
        return refuse(
            cx,
            "a window's partition and ordering take no value: write a literal() or a column",
        );
    }
    let keys = plan.plain().or_else(|failure| throw(cx, failure))?;
    Ok(Part::Over(if which == "by" {
        over.by(keys)
    } else {
        over.sort_by(keys)
    })
    .into())
}

/// A frame bound: a whole number of rows or values from the current row, or
/// `null` for unbounded.
fn bound_at(cx: &mut FunctionContext, index: usize) -> NeonResult<Option<i64>> {
    let value = arg(cx, index);
    if value.is_a::<JsNull, _>(cx) {
        return Ok(None);
    }
    super::super::schema::integer(cx, value, "a frame bound").map(Some)
}

/// `pipelineOverFrame(over, "rows" | "range", start, end)`.
fn over_frame(cx: &mut FunctionContext) -> NeonResult<Node> {
    let over = over_at(cx, 0)?;
    let kind = super::super::args::choice(cx, 1, "a frame", &["rows", "range"])?;
    let start = bound_at(cx, 2)?;
    let end = bound_at(cx, 3)?;
    Ok(Part::Over(if kind == "rows" {
        over.rows(start, end)
    } else {
        over.range(start, end)
    })
    .into())
}
