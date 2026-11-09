//! A pipeline's stages over recipes. A stage whose expressions bind nothing
//! takes them as pgorm's plain transform does; one that binds, a value an
//! operand was given or a binder function's placeholders, takes them through
//! the transform's `_with` form, lowering them inside the binder closure.

use std::sync::Arc;

use pgorm::{pgorm_query::Value, pipeline as pl};

use super::{Lowering, Recipe};
use crate::Failure;

/// The most expressions a stage that binds takes: pgorm's `_with` closures
/// return a fixed-size array, which a binding dispatches up to this size.
pub const BOUND_LIST: usize = 32;

/// A stage's expressions and the values its binder function bound, if it had
/// one.
#[derive(Debug, Default)]
pub struct Plan {
    /// The stage's expressions, in order.
    pub recipes: Vec<Arc<Recipe>>,
    /// What the stage's binder function bound, in the order it bound them.
    pub values: Vec<Value>,
}

/// The list-taking stages that give a pipeline back.
#[derive(Debug)]
pub enum Stage {
    /// `derive`: the expressions added as columns.
    Derive,
    /// `select`: the expressions as the whole projection.
    Select,
    /// `sort`: the rows ordered by the expressions.
    Sort,
    /// `window`: the expressions computed over the window given.
    Window(pl::Over),
}

/// A lowering that could not finish, which no input should cause.
fn unlowered() -> Failure {
    Failure::Internal("a pipeline expression could not be lowered".to_owned())
}

impl Plan {
    /// Whether lowering the plan binds anything, so the stage needs its
    /// binder form.
    pub fn binds(&self) -> bool {
        !self.values.is_empty() || self.recipes.iter().any(|recipe| recipe.binds())
    }

    /// The expressions, lowered with no binder, for a plan that binds
    /// nothing.
    pub fn plain(&self) -> Result<Vec<pl::Expr<'static>>, Failure> {
        let mut with = Lowering {
            binder: None,
            bound: &[],
        };
        self.recipes
            .iter()
            .map(|recipe| recipe.lower(&mut with))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(unlowered)
    }

    /// The expressions as the `[Expr; N]` a `_with` closure returns, lowered
    /// in the stage's binder closure: the binder function's values bound
    /// first, in the order it bound them, then each operand's own value as
    /// the expression reaches it. `failed` notes a lowering that could not
    /// finish, which one scope minting both rules out.
    // [spec:pgorm:req:bindings.pipeline-binder]
    // [spec:pgorm:req:bindings.operands]
    fn lower<'brand, const N: usize>(
        &self,
        binder: &mut pl::Binder<'brand>,
        failed: &mut bool,
    ) -> [pl::Expr<'brand>; N] {
        let bound: Vec<_> = self
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
            .collect::<Option<Vec<_>>>()
            .and_then(|all| <[_; N]>::try_from(all).ok())
            .unwrap_or_else(|| {
                *failed = true;
                std::array::from_fn(|_| pl::null())
            })
    }

    /// The plan's one expression, for a stage that takes one.
    fn single(&self) -> Result<[pl::Expr<'static>; 1], Failure> {
        <[_; 1]>::try_from(self.plain()?).map_err(|_| unlowered())
    }
}

/// `$call::<N>(..)` for the plan's length `N`, up to [`BOUND_LIST`]; `None`
/// past it.
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

/// The stage's result: a list past the dispatched bound refused, and a
/// lowering that could not finish reported.
// [spec:pgorm:req:bindings.pipeline-binder]
fn finished<T>(result: Option<T>, failed: bool, count: usize) -> Result<T, Failure> {
    match result {
        None => Err(Failure::Construction(format!(
            "a pipeline stage that binds takes at most {BOUND_LIST} expressions, not {count}"
        ))),
        Some(_) if failed => Err(unlowered()),
        Some(result) => Ok(result),
    }
}

fn list_n<const N: usize>(
    pipeline: pl::Pipeline,
    stage: Stage,
    plan: &Plan,
    failed: &mut bool,
) -> pl::Pipeline {
    match stage {
        Stage::Derive => pipeline.derive_with(|binder| plan.lower::<N>(binder, failed)),
        Stage::Select => pipeline.select_with(|binder| plan.lower::<N>(binder, failed)),
        Stage::Sort => pipeline.sort_with(|binder| plan.lower::<N>(binder, failed)),
        Stage::Window(over) => pipeline.window_with(over, |binder| plan.lower::<N>(binder, failed)),
    }
}

/// `derive`, `select`, `sort` or `window` of the plan's expressions.
pub fn list(pipeline: pl::Pipeline, stage: Stage, plan: Plan) -> Result<pl::Pipeline, Failure> {
    if !plan.binds() {
        let plain = plan.plain()?;
        return Ok(match stage {
            Stage::Derive => pipeline.derive(plain),
            Stage::Select => pipeline.select(plain),
            Stage::Sort => pipeline.sort(plain),
            Stage::Window(over) => pipeline.window(plain, over),
        });
    }
    let mut failed = false;
    let count = plan.recipes.len();
    let result = arity!(count, list_n, (pipeline, stage, &plan, &mut failed));
    finished(result, failed, count)
}

fn group_n<const N: usize>(pipeline: pl::Pipeline, plan: &Plan, failed: &mut bool) -> pl::Grouped {
    pipeline.group_with(|binder| plan.lower::<N>(binder, failed))
}

/// The pipeline grouped by the plan's keys: pgorm's `group`, or
/// `group_with` when a key binds a value.
pub fn group(pipeline: pl::Pipeline, plan: Plan) -> Result<pl::Grouped, Failure> {
    if !plan.binds() {
        return Ok(pipeline.group(plan.plain()?));
    }
    let mut failed = false;
    let count = plan.recipes.len();
    let result = arity!(count, group_n, (pipeline, &plan, &mut failed));
    finished(result, failed, count)
}

fn aggregate_n<const N: usize>(group: pl::Grouped, plan: &Plan, failed: &mut bool) -> pl::Pipeline {
    group.aggregate_with(|binder| plan.lower::<N>(binder, failed))
}

/// The group's aggregates: pgorm's `aggregate`, or `aggregate_with` when
/// one binds a value.
pub fn aggregate(group: pl::Grouped, plan: Plan) -> Result<pl::Pipeline, Failure> {
    if !plan.binds() {
        return Ok(group.aggregate(plan.plain()?));
    }
    let mut failed = false;
    let count = plan.recipes.len();
    let result = arity!(count, aggregate_n, (group, &plan, &mut failed));
    finished(result, failed, count)
}

/// The pipeline's rows the plan's one predicate keeps: pgorm's `filter`,
/// or `filter_with` when it binds a value; a plan of any other length is
/// refused.
pub fn filter(pipeline: pl::Pipeline, plan: Plan) -> Result<pl::Pipeline, Failure> {
    if !plan.binds() {
        let [predicate] = plan.single()?;
        return Ok(pipeline.filter(predicate));
    }
    let mut failed = false;
    let filtered = pipeline.filter_with(|binder| {
        let [predicate] = plan.lower::<1>(binder, &mut failed);
        predicate
    });
    finished(Some(filtered), failed, 1)
}

/// `join` of `source` on `side`, on the plan's one condition.
pub fn join(
    pipeline: pl::Pipeline,
    side: pl::JoinSide,
    source: pl::Source,
    plan: Plan,
) -> Result<pl::Pipeline, Failure> {
    if !plan.binds() {
        let [on] = plan.single()?;
        return Ok(pipeline.join(side, source, on));
    }
    let mut failed = false;
    let joined = pipeline.join_with(side, source, |binder| {
        let [on] = plan.lower::<1>(binder, &mut failed);
        on
    });
    finished(Some(joined), failed, 1)
}
