//! Tables and the other FROM items: a named table with its schema and alias,
//! a table read through `TABLESAMPLE`, an aliased subquery and set-returning
//! functions read as a relation. A FROM item that is not a named table always
//! has an alias, as PostgreSQL requires, so its columns are qualified by a
//! name the caller chose.

use neon::prelude::*;
use pgorm::pgorm_query::{
    ColumnRef, Expr, FromItem, FunctionCall, NamedTable, SampleMethod, SimpleExpr, TableFunction,
    TableName, TableSample,
};

use super::{
    Node,
    args::{
        absent, arg, choice, flag, list, name, name_at, optional_name, refuse, select_at, this,
    },
};

pub(super) const EXPORTS: &[(&str, super::Build)] = &[
    ("tableNew", table_new),
    ("tableAlias", table_alias),
    ("tableCol", table_col),
    ("tableStar", table_star),
    ("tableSample", table_sample),
    ("tableFunctionNew", table_function_new),
    ("tableFunctionAnd", table_function_and),
    (
        "tableFunctionColumnDefinition",
        table_function_column_definition,
    ),
    (
        "tableFunctionWithOrdinality",
        table_function_with_ordinality,
    ),
    ("tableFunctionColumns", table_function_columns),
    ("tableFunctionAlias", table_function_alias),
    ("fromSubquery", from_subquery),
];

/// `tableNew(name, schema, alias)`.
// [spec:pgorm:req:bindings.select+4]
fn table_new(cx: &mut FunctionContext) -> NeonResult<Node> {
    let name = name_at(cx, 0)?;
    let schema = arg(cx, 1);
    let name = match optional_name(cx, schema)? {
        Some(schema) => TableName::SchemaTable(schema, name),
        None => TableName::Table(name),
    };
    let alias = arg(cx, 2);
    let mut table = NamedTable::from(name);
    if let Some(alias) = optional_name(cx, alias)? {
        table = table.alias(alias);
    }
    Ok(Node::Table(table))
}

fn table_alias(cx: &mut FunctionContext) -> NeonResult<Node> {
    let Node::Table(table) = this(cx, 0)? else {
        return cx.throw_type_error("expected a Table");
    };
    let alias = name_at(cx, 1)?;
    Ok(Node::Table(table.alias(alias)))
}

/// The column reference a `Table` or FROM item qualifies `column` with: a
/// table's alias if it has one, else its own, schema-qualified, name; another
/// item's alias.
fn qualified(item: &Node, column: Option<pgorm::pgorm_query::Name>) -> Option<ColumnRef> {
    Some(match (item, column) {
        (Node::Table(table), Some(column)) => match (&table.alias, &table.name) {
            (Some(alias), _) => ColumnRef::TableColumn(alias.clone(), column),
            (None, TableName::Table(name)) => ColumnRef::TableColumn(name.clone(), column),
            (None, TableName::SchemaTable(schema, name)) => {
                ColumnRef::SchemaTableColumn(schema.clone(), name.clone(), column)
            }
        },
        (Node::Table(table), None) => ColumnRef::TableAsterisk(table.qualifier().clone()),
        (Node::FromItem(item), Some(column)) => {
            ColumnRef::TableColumn(item.qualifier().clone(), column)
        }
        (Node::FromItem(item), None) => ColumnRef::TableAsterisk(item.qualifier().clone()),
        _ => return None,
    })
}

/// `tableCol(item, name)`: one of a table's or FROM item's columns.
fn table_col(cx: &mut FunctionContext) -> NeonResult<Node> {
    let item = this(cx, 0)?;
    let column = name_at(cx, 1)?;
    match qualified(&item, Some(column)) {
        Some(column) => Ok(Node::Expr(Expr::col(column).into())),
        None => cx.throw_type_error("expected a Table or a FROM item"),
    }
}

/// `tableStar(item)`: every column of a table or FROM item, `"t".*`.
fn table_star(cx: &mut FunctionContext) -> NeonResult<Node> {
    let item = this(cx, 0)?;
    match qualified(&item, None) {
        Some(column) => Ok(Node::Expr(Expr::col(column).into())),
        None => cx.throw_type_error("expected a Table or a FROM item"),
    }
}

/// `fromSubquery(select, alias)`: `(SELECT ..) AS "alias"`.
// [spec:pgorm:req:bindings.from-subqueries]
fn from_subquery(cx: &mut FunctionContext) -> NeonResult<Node> {
    let select = select_at(cx, 0)?;
    let alias = name_at(cx, 1)?;
    Ok(Node::FromItem(FromItem::SubQuery(select, alias)))
}

/// A sampling argument: any JavaScript number, bound and left to the server
/// to judge, whose refusal a percentage outside 0 to 100 is (`2202H`).
fn sample_number(cx: &mut FunctionContext, index: usize, what: &str) -> NeonResult<f64> {
    let value = arg(cx, index);
    match value.downcast::<JsNumber, _>(cx) {
        Ok(number) => Ok(number.value(cx)),
        Err(_) => cx.throw_type_error(format!("a sample's {what} is a number")),
    }
}

/// `tableSample(table, method, percentage, seed)`: the table read through
/// `TABLESAMPLE BERNOULLI` or `SYSTEM`, its percentage and optional
/// `REPEATABLE` seed bound. The table keeps its schema and alias, the name
/// the item answers to.
// [spec:pgorm:req:bindings.select+4]
fn table_sample(cx: &mut FunctionContext) -> NeonResult<Node> {
    let Node::Table(table) = this(cx, 0)? else {
        return cx.throw_type_error("only a Table is sampled");
    };
    let method = match choice(cx, 1, "a sampling method", &["bernoulli", "system"])? {
        "bernoulli" => SampleMethod::Bernoulli,
        _ => SampleMethod::System,
    };
    let percentage = sample_number(cx, 2, "percentage")?;
    let mut sample = TableSample::new(table, method, percentage);
    let seed = arg(cx, 3);
    if !absent(cx, seed) {
        sample = sample.repeatable(sample_number(cx, 3, "seed")?);
    }
    Ok(Node::FromItem(sample.into()))
}

/// A call a table function reads: a function call `call()` made, and no
/// other expression, since PostgreSQL's `func_table` takes only calls.
fn table_call<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<FunctionCall> {
    match super::args::node(cx, value) {
        Some(Node::Expr(SimpleExpr::FunctionCall(call))) => Ok(call),
        Some(Node::Expr(_)) => refuse(
            cx,
            "a table function reads a function call, not an expression",
        ),
        Some(other) => {
            let what = other.describe();
            cx.throw_type_error(format!(
                "a table function reads a function call, not {what}"
            ))
        }
        None => cx.throw_type_error("a table function reads a function call: call(name, ..)"),
    }
}

fn table_function_at(cx: &mut FunctionContext, index: usize) -> NeonResult<TableFunction> {
    match this(cx, index)? {
        Node::TableFunction(function) => Ok(function),
        other => {
            let what = other.describe();
            cx.throw_type_error(format!("expected a TableFunction, got {what}"))
        }
    }
}

/// `tableFunctionNew(call, rowsFrom)`: one set-returning call, `f(..)`, or
/// `ROWS FROM (f(..))` when `rowsFrom` says — `TableFunction::new` or
/// `TableFunction::rows_from`.
// [spec:pgorm:req:bindings.select+4]
fn table_function_new(cx: &mut FunctionContext) -> NeonResult<Node> {
    let call = arg(cx, 0);
    let call = table_call(cx, call)?;
    let function = if flag(cx, 1)? {
        TableFunction::rows_from(call)
    } else {
        TableFunction::new(call)
    };
    Ok(Node::TableFunction(function))
}

/// `tableFunctionAnd(function, call)`: another call read beside the others.
// [spec:pgorm:req:bindings.select+4]
fn table_function_and(cx: &mut FunctionContext) -> NeonResult<Node> {
    let function = table_function_at(cx, 0)?;
    let call = arg(cx, 1);
    let call = table_call(cx, call)?;
    Ok(Node::TableFunction(function.and(call)))
}

/// `tableFunctionColumnDefinition(function, name, type)`: a column of the
/// `record` the call added last returns, its type as `DataType` arguments
/// take one.
// [spec:pgorm:req:bindings.select+4]
fn table_function_column_definition(cx: &mut FunctionContext) -> NeonResult<Node> {
    let function = table_function_at(cx, 0)?;
    let column = name_at(cx, 1)?;
    let column_type = arg(cx, 2);
    let column_type = super::data_type::column_type(cx, column_type)?;
    Ok(Node::TableFunction(
        function.column_definition(column, column_type),
    ))
}

fn table_function_with_ordinality(cx: &mut FunctionContext) -> NeonResult<Node> {
    let function = table_function_at(cx, 0)?;
    Ok(Node::TableFunction(function.with_ordinality()))
}

/// `tableFunctionColumns(function, names)`: the names the alias gives the
/// relation's columns, replacing any given before.
// [spec:pgorm:req:bindings.select+4]
fn table_function_columns(cx: &mut FunctionContext) -> NeonResult<Node> {
    let function = table_function_at(cx, 0)?;
    let mut names = Vec::new();
    for column in list(cx, 1)? {
        names.push(name(cx, column)?);
    }
    Ok(Node::TableFunction(function.columns(names)))
}

/// `tableFunctionAlias(function, alias)`: the relation as the FROM item the
/// FROM list and the joins take.
// [spec:pgorm:req:bindings.select+4]
fn table_function_alias(cx: &mut FunctionContext) -> NeonResult<Node> {
    let function = table_function_at(cx, 0)?;
    let alias = name_at(cx, 1)?;
    Ok(Node::FromItem(function.alias(alias)))
}

/// The FROM item a `Table` or FROM-item argument stands for.
pub(super) fn from_item<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<FromItem> {
    match super::args::node(cx, value) {
        Some(Node::Table(table)) => Ok(FromItem::Table(table)),
        Some(Node::FromItem(item)) => Ok(item),
        Some(other) => {
            let what = other.describe();
            cx.throw_type_error(format!("a FROM item is a Table or a FromItem, not {what}"))
        }
        None => cx.throw_type_error("a FROM item is a Table or a FromItem"),
    }
}
