//! A binder's scope: the values one `With` function binds, and the brand its
//! placeholders carry.
//!
//! pgorm brands a placeholder with its binder closure's lifetime, so one
//! cannot leave the stage that minted it. JavaScript has no lifetimes, so the
//! scope is an object instead: open while its function runs, closed when the
//! function returns or throws, and taken by the one stage the function's
//! result goes to. An expression remembers the scope of the placeholders in
//! it, and each place one could escape to checks that scope.

use crate::errors::FailureJs as _;
use std::sync::{Arc, Mutex};

use neon::prelude::*;
use pgorm::pgorm_query::Value;

use super::{
    super::{
        Node,
        args::{arg, refuse},
    },
    Part, PlExpr,
    recipe::Recipe,
};
use crate::{
    codec::Codec,
    errors::Failure,
    values::{Tag, read},
};

/// PostgreSQL's limit on a statement's bound parameters.
const MAX_PARAMETERS: usize = 65_535;

#[derive(Debug)]
struct State {
    open: bool,
    taken: bool,
    values: Vec<Value>,
}

/// The scope of one call of a `With` function.
#[derive(Debug)]
pub(crate) struct Scope(Mutex<State>);

/// Throw a `LifecycleError`: a binder or a placeholder used where its scope
/// does not reach.
pub(super) fn lifecycle<'cx, T>(cx: &mut Cx<'cx>, message: impl Into<String>) -> NeonResult<T> {
    let error = Failure::Lifecycle(message.into()).into_js(cx)?;
    cx.throw(error)
}

impl Scope {
    fn state<'cx>(&self, cx: &mut Cx<'cx>) -> NeonResult<std::sync::MutexGuard<'_, State>> {
        match self.0.lock() {
            Ok(state) => Ok(state),
            Err(_) => {
                let error =
                    Failure::Internal("a binder's scope is poisoned".to_owned()).into_js(cx)?;
                cx.throw(error)
            }
        }
    }

    /// Whether the scope's function is still running.
    pub(super) fn is_open<'cx>(&self, cx: &mut Cx<'cx>) -> NeonResult<bool> {
        Ok(self.state(cx)?.open)
    }

    /// The values the scope bound, for the one stage that takes its
    /// function's result, once that function has returned.
    pub(super) fn take<'cx>(&self, cx: &mut Cx<'cx>) -> NeonResult<Vec<Value>> {
        let mut state = self.state(cx)?;
        if state.open || state.taken {
            drop(state);
            return lifecycle(
                cx,
                "a binder's values go to the one stage its function returns to",
            );
        }
        state.taken = true;
        Ok(std::mem::take(&mut state.values))
    }
}

pub(super) const EXPORTS: &[(&str, super::super::Build)] = &[
    ("pipelineScopeOpen", scope_open),
    ("pipelineScopeClose", scope_close),
    ("pipelineBind", bind),
];

pub(super) fn scope_at<'cx>(cx: &mut FunctionContext<'cx>, index: usize) -> NeonResult<Arc<Scope>> {
    let value = arg(cx, index);
    match super::super::args::node(cx, value) {
        Some(Node::Pipeline(part)) => match *part {
            Part::Scope(scope) => Ok(scope),
            _ => cx.throw_type_error("expected a binder's scope"),
        },
        _ => cx.throw_type_error("expected a binder's scope"),
    }
}

fn scope_open(_: &mut FunctionContext) -> NeonResult<Node> {
    let scope = Scope(Mutex::new(State {
        open: true,
        taken: false,
        values: Vec::new(),
    }));
    Ok(Part::Scope(Arc::new(scope)).into())
}

/// `pipelineScopeClose(scope)`: the function returned or threw; nothing more
/// binds in it.
// [spec:pgorm:req:bindings.pipeline-binder]
fn scope_close(cx: &mut FunctionContext) -> NeonResult<Node> {
    let scope = scope_at(cx, 0)?;
    scope.state(cx)?.open = false;
    Ok(Part::Scope(scope).into())
}

/// A value to bind: any parameter, inferred or declared with `Value`, but none
/// whose kind needs a cast pgorm's pipeline cannot write, or `null`, which has
/// no kind.
// [spec:pgorm:req:bindings.operands]
pub(super) fn bindable<'cx>(cx: &mut Cx<'cx>, data: Handle<'cx, JsValue>) -> NeonResult<Value> {
    let codec = Codec::get(cx)?;
    let Some(tagged) = read::infer(cx, codec, data)? else {
        return refuse(
            cx,
            "null has no kind to bind in a pipeline: write literal(null) or isNull(), or bind \
             Value.null(kind)",
        );
    };
    let value = tagged.datum;
    match tagged.tag {
        Tag::Enum(_) | Tag::Created(_) => refuse(
            cx,
            "pgorm's pipeline has no cast to an enum or a created range: bind the text for the \
             server to infer",
        ),
        Tag::Array(element) if matches!(*element, Tag::Enum(_) | Tag::Created(_)) => refuse(
            cx,
            "pgorm's pipeline has no cast to an enum array: bind the text for the server to infer",
        ),
        _ => Ok(value),
    }
}

/// `pipelineBind(scope, value)`: one placeholder for one value, branded with
/// the scope, while its function runs.
// [spec:pgorm:req:bindings.pipeline-binder]
fn bind(cx: &mut FunctionContext) -> NeonResult<Node> {
    let scope = scope_at(cx, 0)?;
    let value = arg(cx, 1);
    let value = bindable(cx, value)?;
    let mut state = scope.state(cx)?;
    if !state.open {
        drop(state);
        return lifecycle(
            cx,
            "a binder binds only while its function runs; this one has returned",
        );
    }
    if state.values.len() >= MAX_PARAMETERS {
        drop(state);
        return refuse(
            cx,
            format!("a pipeline binds at most {MAX_PARAMETERS} values"),
        );
    }
    state.values.push(value);
    let index = state.values.len() - 1;
    drop(state);
    Ok(Part::Expr(PlExpr {
        recipe: Arc::new(Recipe::Bound(index)),
        scope: Some(scope),
    })
    .into())
}
