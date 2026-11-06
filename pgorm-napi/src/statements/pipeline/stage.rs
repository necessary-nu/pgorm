//! A pipeline's sources and stages. A stage whose expressions bind nothing
//! takes them as pgorm's plain transform does; one that binds — a value an
//! operand was given, or a `With` function's placeholders — takes them through
//! the transform's `_with` form, lowering them inside the binder closure.

use std::sync::Arc;

use neon::prelude::*;
use pgorm::{
    pgorm_query::{TableName, Value},
    pipeline as pl,
};

use super::{
    super::{
        Node,
        args::{absent, arg, items, node, refuse, this},
    },
    Part, PlExpr, Relation, Source,
    expr::operand,
    recipe::{Lowering, Recipe},
    scope::{Scope, lifecycle, scope_at},
};
use crate::errors::Failure;

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
    ("pipelineOver", over),
    ("pipelineOverKeys", over_keys),
    ("pipelineOverFrame", over_frame),
];

/// The most expressions a bound list stage takes: pgorm's `_with` closures
/// return a fixed-size array, which the binding dispatches up to this size.
const BOUND_LIST: usize = 32;

/// A stage's expressions, the values its `With` function bound, and whether
/// lowering them binds at all.
struct Plan {
    recipes: Vec<Arc<Recipe>>,
    values: Vec<Value>,
}

impl Plan {
    fn binds(&self) -> bool {
        !self.values.is_empty() || self.recipes.iter().any(|recipe| recipe.binds())
    }

    /// The expressions, lowered with no binder: `None` if any binds.
    fn plain(&self) -> Option<Vec<pl::Expr<'static>>> {
        let mut with = Lowering {
            binder: None,
            bound: &[],
        };
        self.recipes
            .iter()
            .map(|recipe| recipe.lower(&mut with))
            .collect()
    }

    /// The expressions, lowered in a stage's binder closure: the `With`
    /// function's values bound first, in the order it bound them, then each
    /// operand's own value as the expression reaches it. `None` only if a
    /// placeholder outran its values, which one scope minting both rules out.
    fn bound<'brand>(&self, binder: &mut pl::Binder<'brand>) -> Option<Vec<pl::Expr<'brand>>> {
        let bound: Vec<pl::Expr<'brand>> = self
            .values
            .iter()
            .map(|value| binder.bind(value.clone()))
            .collect();
        let mut with = Lowering {
            binder: Some(binder),
            bound: &bound,
        };
        self.recipes
            .iter()
            .map(|recipe| recipe.lower(&mut with))
            .collect()
    }
}

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

/// A lowering that could not finish, which no input should cause.
fn unlowered<'cx, T>(cx: &mut Cx<'cx>) -> NeonResult<T> {
    let error =
        Failure::Internal("a pipeline expression could not be lowered".to_owned()).into_js(cx)?;
    cx.throw(error)
}

/// The plan's expressions as the `[Expr; N]` a `_with` closure returns,
/// noting in `failed` a lowering that could not finish.
fn lower_n<'brand, const N: usize>(
    plan: &Plan,
    binder: &mut pl::Binder<'brand>,
    failed: &mut bool,
) -> [pl::Expr<'brand>; N] {
    match plan
        .bound(binder)
        .and_then(|all| <[_; N]>::try_from(all).ok())
    {
        Some(array) => array,
        None => {
            *failed = true;
            std::array::from_fn(|_| pl::null())
        }
    }
}

/// The list-taking stages that give a pipeline back.
enum List {
    Derive,
    Select,
    Sort,
    Window(pl::Over),
}

fn list_n<const N: usize>(
    pipeline: pl::Pipeline,
    stage: List,
    plan: &Plan,
    failed: &mut bool,
) -> pl::Pipeline {
    match stage {
        List::Derive => pipeline.derive_with(|binder| lower_n::<N>(plan, binder, failed)),
        List::Select => pipeline.select_with(|binder| lower_n::<N>(plan, binder, failed)),
        List::Sort => pipeline.sort_with(|binder| lower_n::<N>(plan, binder, failed)),
        List::Window(over) => {
            pipeline.window_with(over, |binder| lower_n::<N>(plan, binder, failed))
        }
    }
}

fn group_n<const N: usize>(pipeline: pl::Pipeline, plan: &Plan, failed: &mut bool) -> pl::Grouped {
    pipeline.group_with(|binder| lower_n::<N>(plan, binder, failed))
}

fn aggregate_n<const N: usize>(
    grouped: pl::Grouped,
    plan: &Plan,
    failed: &mut bool,
) -> pl::Pipeline {
    grouped.aggregate_with(|binder| lower_n::<N>(plan, binder, failed))
}

/// `$call::<N>(..)` for the plan's length `N`, up to the bound the binding
/// dispatches; `None` past it.
macro_rules! arity {
    ($len:expr, $call:ident, $arguments:tt) => {
        arity!(@sizes $len, $call, $arguments; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
            15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32)
    };
    (@sizes $len:expr, $call:ident, $arguments:tt; $($size:literal),+) => {
        match $len {
            $($size => Some($call::<$size> $arguments),)+
            _ => None,
        }
    };
}

/// A bound list's refusal past the dispatched bound.
fn too_long<'cx, T>(cx: &mut Cx<'cx>, count: usize) -> NeonResult<T> {
    refuse(
        cx,
        format!("a stage that binds takes at most {BOUND_LIST} expressions, not {count}"),
    )
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
    if !plan.binds() {
        let Some([predicate]) = plan.plain().and_then(|all| <[_; 1]>::try_from(all).ok()) else {
            return unlowered(cx);
        };
        return Ok(pipeline.filter(predicate).into());
    }
    let mut failed = false;
    let filtered = pipeline.filter_with(|binder| {
        let [predicate] = lower_n::<1>(&plan, binder, &mut failed);
        predicate
    });
    if failed {
        return unlowered(cx);
    }
    Ok(filtered.into())
}

/// A list stage: `stage` over the expressions at `index` and the scope after
/// them, plainly when they bind nothing.
fn list_stage(
    cx: &mut FunctionContext,
    pipeline: pl::Pipeline,
    stage: List,
    index: usize,
) -> NeonResult<Node> {
    let plan = plan_at(cx, index)?;
    if !plan.binds() {
        let Some(plain) = plan.plain() else {
            return unlowered(cx);
        };
        return Ok((match stage {
            List::Derive => pipeline.derive(plain),
            List::Select => pipeline.select(plain),
            List::Sort => pipeline.sort(plain),
            List::Window(over) => pipeline.window(plain, over),
        })
        .into());
    }
    let mut failed = false;
    let count = plan.recipes.len();
    let Some(staged) = arity!(count, list_n, (pipeline, stage, &plan, &mut failed)) else {
        return too_long(cx, count);
    };
    if failed {
        return unlowered(cx);
    }
    Ok(staged.into())
}

/// `pipelineList(pipeline, "derive" | "select" | "sort", expressions, scope)`.
// [spec:pgorm:req:bindings.pipeline+1]
fn list(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pipeline = pipeline_at(cx, 0)?;
    let stage = super::super::args::choice(cx, 1, "a stage", &["derive", "select", "sort"])?;
    let stage = match stage {
        "derive" => List::Derive,
        "select" => List::Select,
        _ => List::Sort,
    };
    list_stage(cx, pipeline, stage, 2)
}

/// `pipelineWindow(pipeline, over, expressions, scope)`.
fn window(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pipeline = pipeline_at(cx, 0)?;
    let over = over_at(cx, 1)?;
    list_stage(cx, pipeline, List::Window(over), 2)
}

/// `pipelineGroup(pipeline, keys, scope)`: a grouping, no relation until it
/// is aggregated.
// [spec:pgorm:req:bindings.pipeline+1]
fn group(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pipeline = pipeline_at(cx, 0)?;
    let plan = plan_at(cx, 1)?;
    let grouped = if plan.binds() {
        let mut failed = false;
        let count = plan.recipes.len();
        let Some(grouped) = arity!(count, group_n, (pipeline, &plan, &mut failed)) else {
            return too_long(cx, count);
        };
        if failed {
            return unlowered(cx);
        }
        grouped
    } else {
        let Some(plain) = plan.plain() else {
            return unlowered(cx);
        };
        pipeline.group(plain)
    };
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
    if !plan.binds() {
        let Some(plain) = plan.plain() else {
            return unlowered(cx);
        };
        return Ok(grouped.aggregate(plain).into());
    }
    let mut failed = false;
    let count = plan.recipes.len();
    let Some(aggregated) = arity!(count, aggregate_n, (grouped, &plan, &mut failed)) else {
        return too_long(cx, count);
    };
    if failed {
        return unlowered(cx);
    }
    Ok(aggregated.into())
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
    if !plan.binds() {
        let Some([on]) = plan.plain().and_then(|all| <[_; 1]>::try_from(all).ok()) else {
            return unlowered(cx);
        };
        return Ok(pipeline.join(side, joined, on).into());
    }
    let mut failed = false;
    let result = pipeline.join_with(side, joined, |binder| {
        let [on] = lower_n::<1>(&plan, binder, &mut failed);
        on
    });
    if failed {
        return unlowered(cx);
    }
    Ok(result.into())
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

/// `pipelineSelectDescribed(pipeline, sources)`: pgorm's `select_described`,
/// each source `[qualifier, columns]` and each column `[name, readAs, array]`,
/// `readAs` the type the column is read as where its record reads it cast,
/// or null.
// [spec:pgorm:req:bindings.pipeline-models]
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
            let column = items(cx, column)?;
            let (Some(name), Some(read_as), Some(array)) =
                (column.first(), column.get(1), column.get(2))
            else {
                return cx.throw_type_error("a described column is [name, readAs, array]");
            };
            let name = super::super::args::name(cx, *name)?;
            let read_as = match absent(cx, *read_as) {
                true => None,
                false => Some(super::super::args::type_name(cx, *read_as)?),
            };
            let array = array.downcast_or_throw::<JsBoolean, _>(cx)?.value(cx);
            described = described.column(
                name,
                read_as.map(|type_name| match array {
                    true => type_name.array(),
                    false => type_name,
                }),
            );
        }
        sources.push(described);
    }
    match pipeline.select_described(sources) {
        Ok(pipeline) => Ok(pipeline.into()),
        Err(error) => refuse(cx, error.to_string()),
    }
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
    let Some(keys) = plan.plain() else {
        return unlowered(cx);
    };
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
