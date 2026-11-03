//! pgorm-query's statements and expressions, built from JavaScript.
//!
//! Every JavaScript builder object owns one [`Node`]: the Rust builder state
//! it stands for, boxed. Builders are immutable — each method clones its
//! receiver's state, applies the Rust builder's own method and boxes the
//! result anew — so a statement or expression can be reused and extended in
//! two directions without one affecting the other, as in pgorm-python. The
//! typestates the Rust builders hold — a MERGE before its first arm, a frame
//! start before its end, a conflict target before its action — are separate
//! variants, so a method one of them lacks is refused rather than emulated.
//!
//! Inspecting a statement and running it build the same state the same way,
//! through [`compile`].

mod args;
mod conflict;
mod copy;
mod data_type;
mod expr;
mod grouping;
mod json;
mod json_table;
mod merge;
#[cfg(test)]
pub(crate) mod parity;
pub(crate) mod pipeline;
mod returning;
mod schema;
mod select;
mod table;
#[cfg(test)]
mod variants;
mod window;
mod with;
mod write;

use neon::{prelude::*, types::Finalize};
use pgorm::pgorm_query::{
    AnyWithClause, ColumnType, Condition, ConflictUpdate, ExtendedWindow, FrameClause, FromItem,
    FunctionCall, GroupingElement, JsonInput, JsonTableColumn, MergeStatement, NamedTable,
    NullOrdering, OnConflict, Order, PendingMerge, Query, SelectStatement, SimpleExpr, Value,
    Values, WindowStatement,
};

use crate::{codec::Codec, rows, values::Tagged};
use args::refuse;
pub(crate) use args::select as embedded_select;
pub(crate) use args::{node, pick, predicate};
pub(crate) use copy::{CopyFrom, CopyTo, copy_from_at, copy_to_at};
pub(crate) use schema::Ddl;

/// A projection: an expression with the name its output column takes.
#[derive(Debug, Clone)]
pub(crate) struct Aliased {
    pub(crate) expr: SimpleExpr,
    pub(crate) alias: pgorm::pgorm_query::Name,
}

/// One ORDER BY item.
#[derive(Debug, Clone)]
pub(crate) struct Ordering {
    pub(crate) expr: SimpleExpr,
    pub(crate) order: Order,
    pub(crate) nulls: Option<NullOrdering>,
}

/// The Rust builder state one JavaScript builder object owns.
#[derive(Debug, Clone)]
pub(crate) enum Node {
    Expr(SimpleExpr),
    Condition(Condition),
    Aliased(Aliased),
    Ordering(Ordering),
    /// The operand of a simple `CASE` before its first arm, which is no
    /// expression yet: PostgreSQL's grammar requires a `WHEN`.
    CaseOperand(SimpleExpr),
    /// `ANY (..)` or `ALL (..)`, which PostgreSQL's grammar reads only as the
    /// right operand of an operator, so no other position takes it.
    Quantified(SimpleExpr),
    Table(NamedTable),
    FromItem(FromItem),
    /// An item of a `GROUP BY` list beyond a bare expression: a set,
    /// `ROLLUP`, `CUBE` or `GROUPING SETS`.
    GroupingElement(GroupingElement),
    Select(SelectStatement),
    With(AnyWithClause),
    Insert(write::Insert),
    Update(write::Update),
    Delete(write::Delete),
    /// A conflict arbiter before its action.
    Arbiter(conflict::Arbiter),
    /// A conflict's `DO UPDATE`, which can take more assignments.
    ConflictUpdate(ConflictUpdate),
    /// A completed conflict action.
    Conflict(OnConflict),
    /// A MERGE before its first WHEN arm, which PostgreSQL refuses.
    PendingMerge(Box<PendingMerge>),
    Merge(Box<MergeStatement>),
    MergeAction(merge::Action),
    DataType(ColumnType),
    /// An operand marked `FORMAT JSON`, read only where SQL/JSON reads JSON.
    JsonInput(JsonInput),
    /// A JSON function's `DEFAULT`, written as a literal.
    JsonDefault(Value),
    JsonTableColumn(JsonTableColumn),
    Window(WindowStatement),
    /// A window built on an earlier entry of a WINDOW clause.
    ExtendedWindow(ExtendedWindow),
    FrameStart(window::Start),
    Frame(FrameClause),
    Windowed(window::Windowed),
    /// A function PostgreSQL computes only over a window, which has no form
    /// but `over`.
    WindowFunction(FunctionCall),
    /// A DDL statement, or a part one is built from.
    Schema(Box<schema::Part>),
    /// pgorm's pipeline state: a pipeline, a grouping, an expression, a
    /// source, a window or a binder's scope.
    Pipeline(Box<pipeline::Part>),
    /// A `COPY … FROM STDIN` or `COPY … TO STDOUT`, which only `copyIn` and
    /// `copyOut` run.
    Copy(Box<copy::CopyStatement>),
}

impl Finalize for Node {}

impl Node {
    /// What the node is, as an error names it.
    pub(crate) fn describe(&self) -> &'static str {
        match self {
            Self::Expr(_) => "an expression",
            Self::Condition(_) => "a Condition",
            Self::Aliased(_) => "an aliased projection",
            Self::Ordering(_) => "an ordering",
            Self::CaseOperand(_) => "a CASE with no WHEN arm",
            Self::Quantified(_) => "an ANY or ALL operand",
            Self::Table(_) => "a Table",
            Self::FromItem(_) => "a FROM item",
            Self::GroupingElement(_) => "a grouping element",
            Self::Select(_) => "a Select",
            Self::With(_) => "a WITH clause",
            Self::Insert(_) => "an INSERT",
            Self::Update(_) => "an UPDATE",
            Self::Delete(_) => "a DELETE",
            Self::Arbiter(_) => "a conflict target with no action",
            Self::ConflictUpdate(_) | Self::Conflict(_) => "a conflict action",
            Self::PendingMerge(_) => "a MERGE with no WHEN arm",
            Self::Merge(_) => "a MERGE",
            Self::MergeAction(_) => "a MERGE action",
            Self::DataType(_) => "a DataType",
            Self::JsonInput(_) => "a FORMAT JSON input",
            Self::JsonDefault(_) => "a JSON DEFAULT",
            Self::JsonTableColumn(_) => "a JSON_TABLE column",
            Self::Window(_) => "a Window",
            Self::ExtendedWindow(_) => "an ExtendedWindow",
            Self::FrameStart(_) => "a frame's start",
            Self::Frame(_) => "a frame",
            Self::Windowed(_) => "a windowed call",
            Self::WindowFunction(_) => "a window function with no window",
            Self::Schema(part) => part.describe(),
            Self::Pipeline(part) => part.describe(),
            Self::Copy(copy) => copy.describe(),
        }
    }
}

/// A builder export: it reads its arguments and makes the node the
/// JavaScript object it returns will own.
type Build = fn(&mut FunctionContext) -> NeonResult<Node>;

pub(crate) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("statementInspect", inspect)?;
    for exports in [
        expr::EXPORTS,
        select::EXPORTS,
        grouping::EXPORTS,
        table::EXPORTS,
        with::EXPORTS,
        write::EXPORTS,
        conflict::EXPORTS,
        returning::EXPORTS,
        merge::EXPORTS,
        data_type::EXPORTS,
        json::EXPORTS,
        json_table::EXPORTS,
        window::EXPORTS,
        copy::EXPORTS,
    ] {
        for &(name, build) in exports {
            cx.export_function(name, move |mut cx| {
                let node = build(&mut cx)?;
                Ok(cx.boxed(node))
            })?;
        }
    }
    let added = schema::EXPORTS.iter().chain(pipeline::EXPORTS);
    for &(name, build) in added.copied().flatten() {
        cx.export_function(name, move |mut cx| {
            let node = build(&mut cx)?;
            Ok(cx.boxed(node))
        })?;
    }
    Ok(())
}

/// PostgreSQL's limit on a statement's bound parameters: the protocol counts
/// them in an unsigned 16-bit field.
const MAX_PARAMETERS: usize = 65_535;

/// The SQL and values a statement node builds, through the same validated
/// state whether it is inspected or run. A node that is no complete
/// statement is refused, and so is a SELECT whose tie limit or WINDOW
/// clause PostgreSQL would refuse: rendering writes either regardless, and
/// the ORDER BY or window entry they turn on may arrive later, so this is
/// the first point the statement is final.
// [spec:pgorm:req:bindings.statements]
// [spec:pgorm:req:bindings.select+2]
// [spec:pgorm:req:bindings.windows]
pub(crate) fn compile<'cx>(cx: &mut Cx<'cx>, node: &Node) -> NeonResult<(String, Values)> {
    let built = match node {
        Node::Select(select) => match select.check_clauses() {
            Ok(()) => select.build(),
            Err(error) => return refuse(cx, error.to_string()),
        },
        Node::Merge(merge) => merge.build(),
        Node::Pipeline(part) => pipeline::compile(cx, part)?,
        Node::Schema(part) => match part.statement() {
            Ok(sql) => (sql, Values(Vec::new())),
            Err(reason) => return refuse(cx, reason),
        },
        Node::PendingMerge(_) => {
            return refuse(
                cx,
                "a MERGE needs a WHEN arm before it can be inspected or run: whenMatched(..), \
                 whenNotMatched(..) or whenNotMatchedBySource(..)",
            );
        }
        // A COPY opens a data flow no statement terminal reads or feeds.
        // [spec:pgorm:req:bindings.copy]
        Node::Copy(copy) => {
            let what = copy.describe();
            return cx.throw_type_error(format!(
                "{what} runs through copyIn(..) or copyOut(..), which carry its data"
            ));
        }
        other => match write::built(other) {
            Some(Ok(built)) => built,
            Some(Err(reason)) => return refuse(cx, reason),
            None => {
                let what = other.describe();
                return cx.throw_type_error(format!("{what} is not a statement to run"));
            }
        },
    };
    limited(cx, built)
}

/// `built`, unless it binds more values than PostgreSQL takes.
fn limited<'cx>(cx: &mut Cx<'cx>, built: (String, Values)) -> NeonResult<(String, Values)> {
    if built.1.0.len() > MAX_PARAMETERS {
        return refuse(
            cx,
            format!(
                "the statement binds {} values, past PostgreSQL's {MAX_PARAMETERS}",
                built.1.0.len()
            ),
        );
    }
    Ok(built)
}

/// `statementInspect(node)`: `[sql, values]`, each value a tagged `Value`. A
/// statement is built as it runs; an expression as the one item of a
/// `SELECT`, and a condition as the `WHERE` of `SELECT TRUE`, as
/// pgorm-python inspects them.
// [spec:pgorm:req:bindings.statements]
fn inspect(mut cx: FunctionContext) -> JsResult<JsArray> {
    let node = cx.argument::<JsBox<Node>>(0)?;
    let (sql, values) = match &**node {
        Node::Expr(expr) => limited(&mut cx, Query::select().expr(expr.clone()).build())?,
        Node::Condition(condition) => {
            let select = Query::select()
                .expr(SimpleExpr::Constant(true.into()))
                .cond_where(condition.clone())
                .to_owned();
            limited(&mut cx, select.build())?
        }
        Node::Copy(copy) => (copy.sql(&mut cx)?, Values(Vec::new())),
        other => compile(&mut cx, other)?,
    };
    let codec = Codec::get(&mut cx)?;
    let list = JsArray::new(&mut cx, values.0.len());
    for (at, value) in values.0.into_iter().enumerate() {
        let value = Tagged::value(value);
        let value = rows::value(&mut cx, codec, value, true)?;
        list.set(&mut cx, u32::try_from(at).unwrap_or(u32::MAX), value)?;
    }
    let pair = JsArray::new(&mut cx, 2);
    let sql = cx.string(sql);
    pair.set(&mut cx, 0, sql)?;
    pair.set(&mut cx, 1, list)?;
    Ok(pair)
}
