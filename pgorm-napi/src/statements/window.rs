//! Windows over pgorm-query's `WindowStatement` and its frame typestate: a
//! start knows which side of the current row it lies on and offers only the
//! ends PostgreSQL's grammar lets follow it, so no frame whose end comes
//! before its start can be built.

use neon::prelude::*;
use pgorm::pgorm_query::{
    ExtendedWindow, FrameClause, FrameExclusion, FrameType, FunctionCall, JsonArrayAgg,
    JsonObjectAgg, Name, OverStatement, SelectStatement, SimpleExpr, SqlJson, WindowDefinition,
    WindowFunction, WindowStatement,
};

use super::{
    Node,
    args::{arg, choice, expressions_at, name, node, operand_at, operands_at, refuse, this},
    select,
};
use crate::values::read;

pub(super) const EXPORTS: &[(&str, super::Build)] = &[
    ("windowNew", window_new),
    ("windowPartitionBy", window_partition_by),
    ("windowOrderBy", window_order_by),
    ("windowFrame", window_frame),
    ("extendedWindowNew", extended_window_new),
    ("extendedWindowOrderBy", extended_window_order_by),
    ("extendedWindowFrame", extended_window_frame),
    ("frameStart", frame_start),
    ("frameEnd", frame_end),
    ("frameExclude", frame_exclude),
    ("exprOver", expr_over),
    ("windowedAlias", windowed_alias),
    ("windowFunction", window_function),
];

/// A frame's start, by the side of the current row it lies on.
#[derive(Debug, Clone)]
pub(crate) enum Start {
    Preceding(pgorm::pgorm_query::FrameStart<pgorm::pgorm_query::FramePreceding>),
    CurrentRow(pgorm::pgorm_query::FrameStart<pgorm::pgorm_query::FrameCurrentRow>),
    Following(pgorm::pgorm_query::FrameStart<pgorm::pgorm_query::FrameFollowing>),
}

/// What `OVER` may follow: a function call or one of SQL/JSON's two
/// aggregates, as pgorm-query's sealed `WindowFunction` has it.
#[derive(Debug, Clone)]
pub(crate) enum Call {
    Function(FunctionCall),
    ArrayAgg(JsonArrayAgg),
    ObjectAgg(JsonObjectAgg),
}

/// The window a call runs over: one written inline, which may build on an
/// entry of the statement's WINDOW clause, or the name of the one the
/// statement declares.
#[derive(Debug, Clone)]
pub(crate) enum Over {
    Inline(WindowDefinition),
    Named(Name),
}

/// A call under `OVER`, a SELECT item.
#[derive(Debug, Clone)]
pub(crate) struct Windowed {
    call: Call,
    over: Over,
    alias: Option<Name>,
}

impl Windowed {
    /// Add this item to a SELECT list through the method its window and
    /// alias call for.
    pub(super) fn project(&self, select: &mut SelectStatement) {
        match &self.call {
            Call::Function(call) => self.project_call(select, call.clone()),
            Call::ArrayAgg(call) => self.project_call(select, call.clone()),
            Call::ObjectAgg(call) => self.project_call(select, call.clone()),
        }
    }

    fn project_call<F: WindowFunction>(&self, select: &mut SelectStatement, call: F) {
        match (&self.over, &self.alias) {
            (Over::Inline(window), None) => select.expr_window(call, window.clone()),
            (Over::Inline(window), Some(alias)) => {
                select.expr_window_as(call, window.clone(), alias.clone())
            }
            (Over::Named(name), None) => select.expr_window_name(call, name.clone()),
            (Over::Named(name), Some(alias)) => {
                select.expr_window_name_as(call, name.clone(), alias.clone())
            }
        };
    }
}

fn window(cx: &mut FunctionContext) -> NeonResult<WindowStatement> {
    match this(cx, 0)? {
        Node::Window(window) => Ok(window),
        other => cx.throw_type_error(format!("expected a Window, got {}", other.describe())),
    }
}

/// `windowNew()`: an empty window, `OVER ()`.
// [spec:pgorm:req:bindings.windows]
fn window_new(_: &mut FunctionContext) -> NeonResult<Node> {
    Ok(Node::Window(WindowStatement::default()))
}

fn window_partition_by(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut window = window(cx)?;
    for expression in expressions_at(cx, 1)? {
        window.add_partition_by(expression);
    }
    Ok(Node::Window(window))
}

fn window_order_by(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut window = window(cx)?;
    let orderings = super::args::orderings_at(cx, 1)?;
    select::order(&mut window, orderings);
    Ok(Node::Window(window))
}

/// The frame at `index`: a whole frame, or a preceding or current-row start
/// standing alone. A following start cannot stand alone, PostgreSQL reading a
/// lone start as running to the current row, which lies behind it.
// [spec:pgorm:req:bindings.windows]
fn frame_at(cx: &mut FunctionContext, index: usize) -> NeonResult<FrameClause> {
    let frame = arg(cx, index);
    match node(cx, frame) {
        Some(Node::Frame(frame)) => Ok(frame),
        Some(Node::FrameStart(Start::Preceding(start))) => Ok(start.into()),
        Some(Node::FrameStart(Start::CurrentRow(start))) => Ok(start.into()),
        Some(Node::FrameStart(Start::Following(_))) => refuse(
            cx,
            "a following start needs an end: andFollowing(..) or andUnboundedFollowing()",
        ),
        _ => {
            cx.throw_type_error("a window's frame is a Frame, or a preceding or current-row start")
        }
    }
}

/// `windowFrame(window, frame)`: the frame, replacing any.
// [spec:pgorm:req:bindings.windows]
fn window_frame(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut window = window(cx)?;
    window.frame(frame_at(cx, 1)?);
    Ok(Node::Window(window))
}

fn extended_window(cx: &mut FunctionContext) -> NeonResult<ExtendedWindow> {
    match this(cx, 0)? {
        Node::ExtendedWindow(window) => Ok(window),
        other => cx.throw_type_error(format!(
            "expected an ExtendedWindow, got {}",
            other.describe()
        )),
    }
}

/// `extendedWindowNew(base)`: a window built on the entry of the WINDOW
/// clause named `base`, copying its PARTITION BY and any ORDER BY. It takes
/// no PARTITION BY of its own, which PostgreSQL refuses there.
// [spec:pgorm:req:bindings.windows]
fn extended_window_new(cx: &mut FunctionContext) -> NeonResult<Node> {
    let base = super::args::name_at(cx, 0)?;
    Ok(Node::ExtendedWindow(ExtendedWindow::new(base)))
}

fn extended_window_order_by(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut window = extended_window(cx)?;
    let orderings = super::args::orderings_at(cx, 1)?;
    select::order(&mut window, orderings);
    Ok(Node::ExtendedWindow(window))
}

/// `extendedWindowFrame(window, frame)`: the frame, replacing any.
// [spec:pgorm:req:bindings.windows]
fn extended_window_frame(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut window = extended_window(cx)?;
    window.frame(frame_at(cx, 1)?);
    Ok(Node::ExtendedWindow(window))
}

/// What a SELECT's `window(name, window)` declares the name as: a `Window`,
/// or an `ExtendedWindow` built on an earlier entry.
// [spec:pgorm:req:bindings.windows]
pub(super) fn definition_at(
    cx: &mut FunctionContext,
    index: usize,
) -> NeonResult<WindowDefinition> {
    let window = arg(cx, index);
    match node(cx, window) {
        Some(Node::Window(window)) => Ok(window.into()),
        Some(Node::ExtendedWindow(window)) => Ok(window.into()),
        _ => cx.throw_type_error("a named window is a Window or an ExtendedWindow"),
    }
}

/// `frameStart(type, start, offset)`: where a frame of a mode begins. There
/// is no unbounded-following start: PostgreSQL's grammar refuses one.
// [spec:pgorm:req:bindings.windows]
fn frame_start(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mode = match choice(cx, 0, "a frame's mode", &["rows", "range", "groups"])? {
        "rows" => FrameType::Rows,
        "range" => FrameType::Range,
        _ => FrameType::Groups,
    };
    let start = match choice(
        cx,
        1,
        "a frame's start",
        &["unboundedPreceding", "preceding", "currentRow", "following"],
    )? {
        "unboundedPreceding" => Start::Preceding(mode.unbounded_preceding()),
        "preceding" => Start::Preceding(mode.preceding(operand_at(cx, 2)?)),
        "currentRow" => Start::CurrentRow(mode.current_row()),
        _ => Start::Following(mode.following(operand_at(cx, 2)?)),
    };
    Ok(Node::FrameStart(start))
}

/// `frameEnd(start, end, offset)`: a whole frame, the end one the start
/// admits — a preceding end only after a preceding start, a current-row end
/// never after a following one.
// [spec:pgorm:req:bindings.windows]
fn frame_end(cx: &mut FunctionContext) -> NeonResult<Node> {
    let Node::FrameStart(start) = this(cx, 0)? else {
        return cx.throw_type_error("a frame's end follows its start");
    };
    let end = choice(
        cx,
        1,
        "a frame's end",
        &["preceding", "currentRow", "following", "unboundedFollowing"],
    )?;
    let frame = match (start, end) {
        (Start::Preceding(start), "preceding") => start.and_preceding(operand_at(cx, 2)?),
        (Start::Preceding(start), "currentRow") => start.and_current_row(),
        (Start::Preceding(start), "following") => start.and_following(operand_at(cx, 2)?),
        (Start::Preceding(start), _) => start.and_unbounded_following(),
        (Start::CurrentRow(start), "currentRow") => start.and_current_row(),
        (Start::CurrentRow(start), "following") => start.and_following(operand_at(cx, 2)?),
        (Start::CurrentRow(start), "unboundedFollowing") => start.and_unbounded_following(),
        (Start::Following(start), "following") => start.and_following(operand_at(cx, 2)?),
        (Start::Following(start), "unboundedFollowing") => start.and_unbounded_following(),
        (_, end) => {
            return refuse(
                cx,
                format!("a frame cannot end {end} its start, before where it begins"),
            );
        }
    };
    Ok(Node::Frame(frame))
}

/// `frameExclude(frame, exclusion)`: `EXCLUDE ..`, of a whole frame or of a
/// preceding or current-row start standing alone.
fn frame_exclude(cx: &mut FunctionContext) -> NeonResult<Node> {
    let frame = this(cx, 0)?;
    let exclusion = match choice(
        cx,
        1,
        "an exclusion",
        &["currentRow", "group", "ties", "noOthers"],
    )? {
        "currentRow" => FrameExclusion::CurrentRow,
        "group" => FrameExclusion::Group,
        "ties" => FrameExclusion::Ties,
        _ => FrameExclusion::NoOthers,
    };
    let frame = match frame {
        Node::Frame(frame) => frame.exclude(exclusion),
        Node::FrameStart(Start::Preceding(start)) => start.exclude(exclusion),
        Node::FrameStart(Start::CurrentRow(start)) => start.exclude(exclusion),
        Node::FrameStart(Start::Following(_)) => {
            return refuse(cx, "a following start needs its end before EXCLUDE");
        }
        _ => return cx.throw_type_error("EXCLUDE belongs to a frame"),
    };
    Ok(Node::Frame(frame))
}

/// The call an expression is, refused unless the grammar puts it before
/// `OVER`: a column, arithmetic, a cast or another SQL/JSON function is not.
fn call_of(cx: &mut FunctionContext, node: Node) -> NeonResult<Call> {
    match node {
        Node::Expr(SimpleExpr::FunctionCall(call)) | Node::WindowFunction(call) => {
            Ok(Call::Function(call))
        }
        Node::Expr(SimpleExpr::SqlJson(json)) => match *json {
            SqlJson::ArrayAgg(aggregate) => Ok(Call::ArrayAgg(aggregate)),
            SqlJson::ObjectAgg(aggregate) => Ok(Call::ObjectAgg(aggregate)),
            _ => refuse(
                cx,
                "OVER follows only a function call or jsonArrayagg / jsonObjectagg",
            ),
        },
        _ => refuse(
            cx,
            "OVER follows only a function call or jsonArrayagg / jsonObjectagg",
        ),
    }
}

/// `exprOver(call, window)`: the call over a `Window` or an `ExtendedWindow`
/// written inline, or over the window a statement names.
// [spec:pgorm:req:bindings.windows]
fn expr_over(cx: &mut FunctionContext) -> NeonResult<Node> {
    let receiver = this(cx, 0)?;
    let call = call_of(cx, receiver)?;
    let window = arg(cx, 1);
    let over = match node(cx, window) {
        Some(Node::Window(window)) => Over::Inline(window.into()),
        Some(Node::ExtendedWindow(window)) => Over::Inline(window.into()),
        Some(other) => {
            let what = other.describe();
            return cx.throw_type_error(format!(
                "OVER takes a Window, an ExtendedWindow or a window's name, not {what}"
            ));
        }
        None => Over::Named(name(cx, window)?),
    };
    Ok(Node::Windowed(Windowed {
        call,
        over,
        alias: None,
    }))
}

fn windowed_alias(cx: &mut FunctionContext) -> NeonResult<Node> {
    let Node::Windowed(windowed) = this(cx, 0)? else {
        return cx.throw_type_error("expected a windowed call");
    };
    let alias = super::args::name_at(cx, 1)?;
    Ok(Node::Windowed(Windowed {
        alias: Some(alias),
        ..windowed
    }))
}

/// `windowFunction(name, args)`: one of PostgreSQL's general-purpose window
/// functions at the argument counts it takes, usable only through `over`, as
/// the server refuses one without a window (`42809`).
// [spec:pgorm:req:bindings.windows]
fn window_function(cx: &mut FunctionContext) -> NeonResult<Node> {
    let name = arg(cx, 0);
    let name = read::text(cx, name, "a window function's name")?;
    let arguments = operands_at(cx, 1)?;
    match pgorm_bindings::window_function(&name, arguments, crate::connect::DIALECT) {
        Ok(function) => Ok(Node::WindowFunction(function)),
        Err(refusal) => refuse(cx, refusal.into_message()),
    }
}
