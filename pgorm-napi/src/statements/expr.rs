//! Expressions and conditions: columns, bound values, operators, predicates,
//! function calls, casts, collations, subscripts, `CASE` and subqueries, each
//! through the pgorm-query builder method of the same meaning.

use neon::prelude::*;
use pgorm::pgorm_query::{
    BinOper, ColumnRef, Condition, Expr, ExprTrait, Func, Name, NullOrdering, Order, SimpleExpr,
    SubQueryStatement,
};

use super::{
    Aliased, Node, Ordering,
    args::{
        self, arg, choice, expression, list, name_at, operand, operand_at, operands_at,
        optional_name, predicate, predicate_at, refuse, select_at, this,
    },
};

pub(super) const EXPORTS: &[(&str, super::Build)] = &[
    ("exprCol", col),
    ("exprBind", bind),
    ("exprLiteral", literal),
    ("exprCall", call),
    ("exprFunc", func),
    ("exprTemplate", template),
    ("exprJson", json),
    ("exprTuple", tuple),
    ("exprSubquery", subquery),
    ("exprQuantified", quantified),
    ("exprBinary", binary),
    ("exprPathExists", path_exists),
    ("exprLogic", logic),
    ("exprUnary", unary),
    ("exprIn", membership),
    ("exprBetween", between),
    ("exprLike", like),
    ("exprText", text),
    ("exprCast", cast),
    ("exprCollate", collate),
    ("exprSubscript", subscript),
    ("exprAlias", alias),
    ("exprOrder", order),
    ("conditionNew", condition_new),
    ("conditionAdd", condition_add),
    ("conditionAddOption", condition_add_option),
    ("conditionNot", condition_not),
    ("caseWhen", case_when),
    ("caseElse", case_else),
    ("caseOf", case_of),
    ("caseOfWhen", case_of_when),
];

/// The receiver at `index` as the expression it must be.
fn receiver<'cx>(cx: &mut FunctionContext<'cx>, index: usize) -> NeonResult<SimpleExpr> {
    let value = arg(cx, index);
    expression(cx, value)
}

/// `exprCol(name, table, schema)`: a column, qualified by a table and its
/// schema when they are given.
// [spec:pgorm:req:bindings.expressions+4]
fn col(cx: &mut FunctionContext) -> NeonResult<Node> {
    let column = name_at(cx, 0)?;
    let table = arg(cx, 1);
    let table = optional_name(cx, table)?;
    let schema = arg(cx, 2);
    let schema = optional_name(cx, schema)?;
    let column = match (schema, table) {
        (Some(schema), Some(table)) => ColumnRef::SchemaTableColumn(schema, table, column),
        (None, Some(table)) => ColumnRef::TableColumn(table, column),
        (None, None) => ColumnRef::Column(column),
        (Some(_), None) => return refuse(cx, "a schema-qualified column needs its table"),
    };
    Ok(Node::Expr(Expr::col(column).into()))
}

/// `exprBind(value)`: a bound value.
// [spec:pgorm:req:bindings.expressions+4]
fn bind(cx: &mut FunctionContext) -> NeonResult<Node> {
    let value = arg(cx, 0);
    let expr = args::bound(cx, value)?;
    Ok(Node::Expr(expr))
}

/// `exprLiteral(value)`: a value written into the SQL text.
// [spec:pgorm:req:bindings.operands]
fn literal(cx: &mut FunctionContext) -> NeonResult<Node> {
    let value = arg(cx, 0);
    let expr = args::literal(cx, value)?;
    Ok(Node::Expr(expr))
}

/// `exprCall(name, args)`: a call to one of the functions pgorm-query has a
/// constructor for. Another name or argument count is refused: the name
/// selects a constructor and never reaches SQL.
// [spec:pgorm:req:bindings.expressions+4]
fn call(cx: &mut FunctionContext) -> NeonResult<Node> {
    let value = arg(cx, 0);
    let name = crate::values::read::text(cx, value, "a function's name")?;
    let args = operands_at(cx, 1)?;
    match pgorm_bindings::call(&name, &args) {
        Ok(call) => Ok(Node::Expr(call)),
        Err(refusal) => refuse(cx, refusal.into_message()),
    }
}

/// `exprFunc(name, args)`: a call of the function `name`, any the server
/// has, through Rust's `Func::named`. The name is an identifier, written bare
/// when it is a lowercase name PostgreSQL does not restrict and quoted
/// otherwise, so it names a function and is never read as syntax.
// [spec:pgorm:req:bindings.named-functions]
fn func(cx: &mut FunctionContext) -> NeonResult<Node> {
    let name = name_at(cx, 0)?;
    let args = operands_at(cx, 1)?;
    Ok(Node::Expr(Func::named(name).args(args).into()))
}

/// `exprTemplate(strings, values)`: a tagged template's pieces, its SQL,
/// with each interpolated value between them, an expression embedded and any
/// other value bound, through pgorm-query's `SqlTemplate::from_pieces`.
// [spec:pgorm:req:bindings.template]
fn template(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pieces = list(cx, 0)?
        .into_iter()
        .map(|piece| crate::values::read::text(cx, piece, "a template's text"))
        .collect::<NeonResult<Vec<_>>>()?;
    let values = operands_at(cx, 1)?;
    match pgorm::pgorm_query::SqlTemplate::from_pieces(&pieces, values) {
        Ok(template) => Ok(Node::Expr(SimpleExpr::Template(template))),
        Err(error) => refuse(cx, error.to_string()),
    }
}

/// `exprJson(expr, operator, argument)`: a `jsonb` accessor or key test
/// through pgorm-query's method of that meaning: `->` and `->>` take a key
/// or an index, bound unless it is an expression; `#>` and `#>>` a path of
/// keys and `?|` and `?&` keys, each bound as one `text[]`; and `?` one key.
// [spec:pgorm:req:bindings.json-operators]
fn json(cx: &mut FunctionContext) -> NeonResult<Node> {
    let expr = Expr::expr(receiver(cx, 0)?);
    let operator = choice(
        cx,
        1,
        "a JSON operator",
        &[
            "getJsonField",
            "castJsonField",
            "getJsonPath",
            "castJsonPath",
            "hasJsonKey",
            "hasAnyJsonKeys",
            "hasAllJsonKeys",
        ],
    )?;
    let keys = |cx: &mut FunctionContext, what: &str| -> NeonResult<Vec<String>> {
        let values = arg(cx, 2);
        let items = args::items(cx, values)?;
        items
            .into_iter()
            .map(|item| crate::values::read::text(cx, item, what))
            .collect()
    };
    Ok(Node::Expr(match operator {
        "getJsonField" => expr.get_json_field(operand_at(cx, 2)?),
        "castJsonField" => expr.cast_json_field(operand_at(cx, 2)?),
        "getJsonPath" => expr.get_json_path(keys(cx, "a JSON path's key")?),
        "castJsonPath" => expr.cast_json_path(keys(cx, "a JSON path's key")?),
        "hasJsonKey" => {
            let key = arg(cx, 2);
            expr.has_json_key(crate::values::read::text(cx, key, "a JSON key")?)
        }
        "hasAnyJsonKeys" => expr.has_any_json_keys(keys(cx, "a JSON key")?),
        _ => expr.has_all_json_keys(keys(cx, "a JSON key")?),
    }))
}

/// `exprTuple(items)`: `(a, b, ..)`, never empty.
fn tuple(cx: &mut FunctionContext) -> NeonResult<Node> {
    let items = operands_at(cx, 0)?;
    if items.is_empty() {
        return refuse(cx, "a tuple has at least one item");
    }
    Ok(Node::Expr(Expr::tuple(items).into()))
}

/// `exprSubquery(kind, select)`: a scalar subquery, `(SELECT ..)`,
/// `EXISTS (SELECT ..)`, or `ARRAY(SELECT ..)`, its one column collected into
/// one array value through `Expr::array`.
// [spec:pgorm:req:bindings.expressions+4]
fn subquery(cx: &mut FunctionContext) -> NeonResult<Node> {
    let kind = choice(cx, 0, "a subquery's kind", &["scalar", "exists", "array"])?;
    let select = select_at(cx, 1)?;
    let expr = match kind {
        "exists" => Expr::exists(select),
        "array" => Expr::array(select),
        _ => SimpleExpr::SubQuery(None, Box::new(SubQueryStatement::SelectStatement(select))),
    };
    Ok(Node::Expr(expr))
}

/// `exprQuantified("any" | "all", select | operand)`: `ANY (..)` or
/// `ALL (..)` over a subquery's rows, pgorm-query's `Expr::any` / `Expr::all`,
/// or over an array, `Func::any` / `Func::all` with the array bound unless it
/// is an expression. It is no expression: only an operator PostgreSQL reads
/// one after takes it, as its right operand.
// [spec:pgorm:req:bindings.quantifiers+4]
fn quantified(cx: &mut FunctionContext) -> NeonResult<Node> {
    let all = choice(cx, 0, "a quantifier", &["any", "all"])? == "all";
    let source = arg(cx, 1);
    let built = if let Some(Node::Select(_)) = args::node(cx, source) {
        let select = args::select(cx, source)?;
        if all {
            Expr::all(select)
        } else {
            Expr::any(select)
        }
    } else {
        let array = operand(cx, source)?;
        if all {
            Func::all(array).into()
        } else {
            Func::any(array).into()
        }
    };
    Ok(Node::Quantified(built))
}

/// The operators of `BINARY` PostgreSQL's grammar takes before `ANY` or `ALL`
/// whose result is a boolean, as the server requires of one (`42809`). LIKE's
/// family and `@?` read theirs in `like` and `path_exists`.
// [spec:pgorm:req:bindings.quantifiers+4]
const QUANTIFIABLE: [&str; 19] = [
    "eq",
    "ne",
    "lt",
    "lte",
    "gt",
    "gte",
    "contains",
    "containedBy",
    "overlaps",
    "strictlyLeftOf",
    "strictlyRightOf",
    "notExtendRightOf",
    "notExtendLeftOf",
    "adjacentTo",
    "regex",
    "notRegex",
    "iregex",
    "notIregex",
    "matches",
];

/// The refusal of `ANY (..)` or `ALL (..)` after `operator`, which
/// PostgreSQL reads none after.
fn unquantifiable<T>(cx: &mut FunctionContext, operator: &str) -> NeonResult<T> {
    refuse(
        cx,
        format!(
            "ANY and ALL follow a comparison, LIKE or ILIKE, a regular-expression match, \
             containment, a range position, matches or pathExists, not {operator:?}"
        ),
    )
}

// [spec:pgorm:req:bindings.ranges]
pub(super) const BINARY: [(&str, BinOper); 26] = [
    ("eq", BinOper::Equal),
    ("ne", BinOper::NotEqual),
    ("lt", BinOper::SmallerThan),
    ("lte", BinOper::SmallerThanOrEqual),
    ("gt", BinOper::GreaterThan),
    ("gte", BinOper::GreaterThanOrEqual),
    ("add", BinOper::Add),
    ("sub", BinOper::Sub),
    ("mul", BinOper::Mul),
    ("div", BinOper::Div),
    ("mod", BinOper::Mod),
    ("concat", BinOper::Concatenate),
    ("isDistinctFrom", BinOper::IsDistinctFrom),
    ("contains", BinOper::Contains),
    ("containedBy", BinOper::Contained),
    ("overlaps", BinOper::Overlap),
    ("strictlyLeftOf", BinOper::LShift),
    ("strictlyRightOf", BinOper::RShift),
    ("notExtendRightOf", BinOper::NotExtendRightOf),
    ("notExtendLeftOf", BinOper::NotExtendLeftOf),
    ("adjacentTo", BinOper::AdjacentTo),
    ("regex", BinOper::Regex),
    ("notRegex", BinOper::NotRegex),
    ("iregex", BinOper::RegexCaseInsensitive),
    ("notIregex", BinOper::NotRegexCaseInsensitive),
    ("matches", BinOper::Matches),
];

/// `exprBinary(expr, operator, operand)`: a comparison or arithmetic, the
/// operand bound unless it is an expression, or `ANY (..)` or `ALL (..)` for
/// an operator PostgreSQL reads one after. `isNotDistinctFrom` is the one
/// operator outside the table, as pgorm-query spells it.
// [spec:pgorm:req:bindings.expressions+4]
fn binary(cx: &mut FunctionContext) -> NeonResult<Node> {
    let left = receiver(cx, 0)?;
    let operator = arg(cx, 1);
    let operator = crate::values::read::string(cx, operator)?;
    let right = arg(cx, 2);
    let right = match args::node(cx, right) {
        Some(Node::Quantified(quantified)) if QUANTIFIABLE.contains(&operator.as_str()) => {
            quantified
        }
        Some(Node::Quantified(_)) => return unquantifiable(cx, &operator),
        _ => operand(cx, right)?,
    };
    let expr = match BINARY.iter().find(|(name, _)| *name == operator) {
        Some((_, op)) => left.binary(*op, right),
        None if operator == "isNotDistinctFrom" => Expr::expr(left).is_not_distinct_from(right),
        None => return refuse(cx, format!("{operator:?} is no binary operator")),
    };
    Ok(Node::Expr(expr))
}

/// `exprPathExists(expr, path)`: `@?`, the path a string bound as text cast
/// to `jsonpath` by pgorm-query's `json_path_exists`, or `ANY (..)` or
/// `ALL (..)` over `jsonpath` values.
// [spec:pgorm:req:bindings.expressions+4]
// [spec:pgorm:req:bindings.quantifiers+4]
fn path_exists(cx: &mut FunctionContext) -> NeonResult<Node> {
    let expr = receiver(cx, 0)?;
    let path = arg(cx, 1);
    if let Some(Node::Quantified(quantified)) = args::node(cx, path) {
        return Ok(Node::Expr(expr.binary(BinOper::JsonPathExists, quantified)));
    }
    let path = crate::values::read::text(cx, path, "a JSON path")?;
    Ok(Node::Expr(Expr::expr(expr).json_path_exists(path)))
}

/// `exprLogic(expr, "and" | "or", other)`: both sides expressions, never a
/// bound value read as a boolean.
fn logic(cx: &mut FunctionContext) -> NeonResult<Node> {
    let left = receiver(cx, 0)?;
    let operator = choice(cx, 1, "a logical operator", &["and", "or"])?;
    let right = receiver(cx, 2)?;
    let expr = if operator == "and" {
        left.and(right)
    } else {
        left.or(right)
    };
    Ok(Node::Expr(expr))
}

/// `exprUnary(expr, operator)`: `NOT`, `IS NULL`, `IS NOT NULL`.
fn unary(cx: &mut FunctionContext) -> NeonResult<Node> {
    let expr = receiver(cx, 0)?;
    let operator = choice(cx, 1, "a unary operator", &["not", "isNull", "isNotNull"])?;
    let expr = match operator {
        "not" => expr.not(),
        "isNull" => Expr::expr(expr).is_null(),
        _ => Expr::expr(expr).is_not_null(),
    };
    Ok(Node::Expr(expr))
}

/// `exprIn(expr, negated, values | select)`: membership in a list — empty
/// included, which pgorm-query renders as a comparison that is always false
/// for `IN` and always true for `NOT IN` — or in a subquery's rows.
// [spec:pgorm:req:bindings.expressions+4]
fn membership(cx: &mut FunctionContext) -> NeonResult<Node> {
    let expr = Expr::expr(receiver(cx, 0)?);
    let negated = args::flag(cx, 1)?;
    let source = arg(cx, 2);
    let built = if source.is_a::<JsArray, _>(cx) {
        let values = operands_at(cx, 2)?;
        if negated {
            expr.is_not_in(values)
        } else {
            expr.is_in(values)
        }
    } else {
        let select = args::select(cx, source)?;
        if negated {
            expr.not_in_subquery(select)
        } else {
            expr.in_subquery(select)
        }
    };
    Ok(Node::Expr(built))
}

/// `exprBetween(expr, negated, symmetric, lower, upper)`.
fn between(cx: &mut FunctionContext) -> NeonResult<Node> {
    let expr = Expr::expr(receiver(cx, 0)?);
    let negated = args::flag(cx, 1)?;
    let symmetric = args::flag(cx, 2)?;
    let lower = operand_at(cx, 3)?;
    let upper = operand_at(cx, 4)?;
    let built = match (negated, symmetric) {
        (false, false) => expr.between(lower, upper),
        (true, false) => expr.not_between(lower, upper),
        (false, true) => expr.between_symmetric(lower, upper),
        (true, true) => expr.not_between_symmetric(lower, upper),
    };
    Ok(Node::Expr(built))
}

/// `exprLike(expr, operator, pattern)`: a pattern match, LIKE's or SIMILAR
/// TO's, over a string or a `LikeExpr`. LIKE's family also takes `ANY (..)`
/// or `ALL (..)` over patterns, which carry no escape, as the grammar reads
/// none after one.
// [spec:pgorm:req:bindings.expressions+4]
// [spec:pgorm:req:bindings.quantifiers+4]
fn like(cx: &mut FunctionContext) -> NeonResult<Node> {
    let left = receiver(cx, 0)?;
    let operator = choice(
        cx,
        1,
        "a pattern operator",
        &[
            "like",
            "notLike",
            "ilike",
            "notIlike",
            "similarTo",
            "notSimilarTo",
        ],
    )?;
    let pattern = arg(cx, 2);
    if let Some(Node::Quantified(quantified)) = args::node(cx, pattern) {
        return quantified_like(cx, left, operator, quantified);
    }
    let pattern = super::like::pattern(cx, pattern)?;
    let expr = Expr::expr(left);
    let built = match operator {
        "like" => expr.like(pattern),
        "notLike" => expr.not_like(pattern),
        "ilike" => expr.ilike(pattern),
        "notIlike" => expr.not_ilike(pattern),
        "similarTo" => expr.similar_to(pattern),
        _ => expr.not_similar_to(pattern),
    };
    Ok(Node::Expr(built))
}

fn quantified_like(
    cx: &mut FunctionContext,
    left: SimpleExpr,
    operator: &str,
    quantified: SimpleExpr,
) -> NeonResult<Node> {
    let operator = match operator {
        "like" => BinOper::Like,
        "notLike" => BinOper::NotLike,
        "ilike" => BinOper::ILike,
        "notIlike" => BinOper::NotILike,
        other => return unquantifiable(cx, other),
    };
    Ok(Node::Expr(left.binary(operator, quantified)))
}

/// `exprText(expr, helper, text)`: a test that reads its argument as text,
/// `%`, `_` and backslashes included, through PostgreSQL's own functions —
/// `starts_with`, `strpos` and `right` — so there is no escaping to get wrong.
// [spec:pgorm:req:bindings.expressions+4]
fn text(cx: &mut FunctionContext) -> NeonResult<Node> {
    let expr = receiver(cx, 0)?;
    let helper = choice(
        cx,
        1,
        "a text test",
        &["startsWith", "endsWith", "containsText"],
    )?;
    let text = operand_at(cx, 2)?;
    let built = match helper {
        "startsWith" => Func::starts_with(expr, text).into(),
        "containsText" => {
            let position = Func::named(Name::runtime("strpos")).args([expr, text]);
            Expr::expr(position).gt(SimpleExpr::Constant(0i32.into()))
        }
        _ => {
            let suffix = Func::named(Name::runtime("right"))
                .args([expr, Func::char_length(text.clone()).into()]);
            Expr::expr(suffix).eq(text)
        }
    };
    Ok(Node::Expr(built))
}

/// `exprCast(expr, type, array)`: `CAST(expr AS type)`, the type named by
/// identifier and quoted.
// [spec:pgorm:req:bindings.expressions+4]
fn cast(cx: &mut FunctionContext) -> NeonResult<Node> {
    let expr = receiver(cx, 0)?;
    let named = arg(cx, 1);
    let named = args::type_name(cx, named)?;
    let array = args::flag(cx, 2)?;
    let named = if array { named.array() } else { named };
    Ok(Node::Expr(expr.cast_as_type(named)))
}

// [spec:pgorm:req:bindings.collations]
/// `exprCollate(expr, collation, schema)`: `(expr COLLATE "collation")`.
fn collate(cx: &mut FunctionContext) -> NeonResult<Node> {
    let expr = Expr::expr(receiver(cx, 0)?);
    let collation = name_at(cx, 1)?;
    let schema = arg(cx, 2);
    let built = match optional_name(cx, schema)? {
        Some(schema) => expr.collate((schema, collation)),
        None => expr.collate(collation),
    };
    Ok(Node::Expr(built))
}

/// `exprSubscript(expr, index)` or `exprSubscript(expr, lower, upper, true)`:
/// an array element, `a[i]`, or a slice, `a[l:u]`, either bound omissible
/// with `null`.
fn subscript(cx: &mut FunctionContext) -> NeonResult<Node> {
    let expr = Expr::expr(receiver(cx, 0)?);
    let slice = arg(cx, 3);
    if !slice.is_a::<JsBoolean, _>(cx) {
        let index = operand_at(cx, 1)?;
        return Ok(Node::Expr(expr.index(index)));
    }
    let lower = arg(cx, 1);
    let lower = if args::absent(cx, lower) {
        None
    } else {
        Some(operand(cx, lower)?)
    };
    let upper = arg(cx, 2);
    let upper = if args::absent(cx, upper) {
        None
    } else {
        Some(operand(cx, upper)?)
    };
    let built = match (lower, upper) {
        (Some(lower), Some(upper)) => expr.slice(lower, upper),
        (Some(lower), None) => expr.slice_from(lower),
        (None, Some(upper)) => expr.slice_to(upper),
        (None, None) => expr.subscript(pgorm::pgorm_query::Subscript::Slice(None, None)),
    };
    Ok(Node::Expr(built))
}

/// `exprAlias(expr, name)`: a projection, `expr AS "name"`.
fn alias(cx: &mut FunctionContext) -> NeonResult<Node> {
    let expr = receiver(cx, 0)?;
    let alias = name_at(cx, 1)?;
    Ok(Node::Aliased(Aliased { expr, alias }))
}

/// `exprOrder(expr, "asc" | "desc", nulls)`: an ordering, with `NULLS FIRST`
/// or `NULLS LAST` when `nulls` says.
fn order(cx: &mut FunctionContext) -> NeonResult<Node> {
    let expr = receiver(cx, 0)?;
    let direction = choice(cx, 1, "a direction", &["asc", "desc"])?;
    let nulls = arg(cx, 2);
    let nulls = if args::absent(cx, nulls) {
        None
    } else {
        Some(match choice(cx, 2, "nulls", &["first", "last"])? {
            "first" => NullOrdering::First,
            _ => NullOrdering::Last,
        })
    };
    let order = if direction == "asc" {
        Order::Asc
    } else {
        Order::Desc
    };
    Ok(Node::Ordering(Ordering { expr, order, nulls }))
}

/// `conditionNew("all" | "any", items)`: pgorm-query's `Condition::all` or
/// `any` over the items, an empty one being true or false as Rust has it.
// [spec:pgorm:req:bindings.expressions+4]
fn condition_new(cx: &mut FunctionContext) -> NeonResult<Node> {
    let kind = choice(cx, 0, "a condition's kind", &["all", "any"])?;
    let mut condition = if kind == "all" {
        Condition::all()
    } else {
        Condition::any()
    };
    for item in list(cx, 1)? {
        condition = condition.add(predicate(cx, item)?);
    }
    Ok(Node::Condition(condition))
}

fn condition_receiver<'cx>(cx: &mut FunctionContext<'cx>) -> NeonResult<Condition> {
    match this(cx, 0)? {
        Node::Condition(condition) => Ok(condition),
        other => cx.throw_type_error(format!("expected a Condition, got {}", other.describe())),
    }
}

/// `conditionAdd(condition, item)`: a new condition with `item` added.
fn condition_add(cx: &mut FunctionContext) -> NeonResult<Node> {
    let condition = condition_receiver(cx)?;
    let item = predicate_at(cx, 1)?;
    let condition = condition.add(item);
    Ok(Node::Condition(condition))
}

/// `conditionAddOption(condition, item)`: a new condition with `item`
/// added, or the condition as it is where `item` is null or undefined, as
/// Rust's `Condition::add_option` adds an optional condition.
// [spec:pgorm:req:bindings.condition-option]
fn condition_add_option(cx: &mut FunctionContext) -> NeonResult<Node> {
    let condition = condition_receiver(cx)?;
    let item = arg(cx, 1);
    let item = match args::absent(cx, item) {
        true => None,
        false => Some(predicate_at(cx, 1)?),
    };
    Ok(Node::Condition(condition.add_option(item)))
}

/// `conditionNot(condition)`: the condition negated.
fn condition_not(cx: &mut FunctionContext) -> NeonResult<Node> {
    let condition = condition_receiver(cx)?;
    Ok(Node::Condition(condition.not()))
}

/// `caseWhen(case, condition, result)`: the searched `CASE` with one more
/// arm, `case` `null` for the first.
// [spec:pgorm:req:bindings.case]
fn case_when(cx: &mut FunctionContext) -> NeonResult<Node> {
    let case = arg(cx, 0);
    let case = if args::absent(cx, case) {
        None
    } else {
        Some(this(cx, 0)?)
    };
    let condition = predicate_at(cx, 1)?;
    let result = operand_at(cx, 2)?;
    let built: SimpleExpr = match case {
        None => Expr::case(condition, result).into(),
        Some(Node::Expr(SimpleExpr::Case(statement))) => statement.case(condition, result).into(),
        Some(other) => {
            let what = other.describe();
            return cx.throw_type_error(format!("expected a searched CASE, got {what}"));
        }
    };
    Ok(Node::Expr(built))
}

/// `caseOfWhen(case, value, result)`: the simple `CASE` with one more arm,
/// `case` being its operand alone for the first.
fn case_of_when(cx: &mut FunctionContext) -> NeonResult<Node> {
    let case = this(cx, 0)?;
    let value = operand_at(cx, 1)?;
    let result = operand_at(cx, 2)?;
    let built: SimpleExpr = match case {
        Node::CaseOperand(operand) => Expr::case_of(operand).when(value, result).into(),
        Node::Expr(SimpleExpr::SimpleCase(statement)) => statement.when(value, result).into(),
        other => {
            let what = other.describe();
            return cx.throw_type_error(format!("expected a simple CASE, got {what}"));
        }
    };
    Ok(Node::Expr(built))
}

/// `caseElse(case, result)`: the `CASE`'s `ELSE`, after which it takes no arm.
fn case_else(cx: &mut FunctionContext) -> NeonResult<Node> {
    let result = operand_at(cx, 1)?;
    let built: SimpleExpr = match this(cx, 0)? {
        Node::Expr(SimpleExpr::Case(statement)) => statement.finally(result).into(),
        Node::Expr(SimpleExpr::SimpleCase(statement)) => statement.finally(result).into(),
        other => {
            return cx.throw_type_error(format!("expected a CASE, got {}", other.describe()));
        }
    };
    Ok(Node::Expr(built))
}

/// `caseOf(operand)` begins a simple `CASE`, which has no arm and so is no
/// expression until `caseOfWhen` gives it one.
fn case_of(cx: &mut FunctionContext) -> NeonResult<Node> {
    let operand = operand_at(cx, 0)?;
    Ok(Node::CaseOperand(operand))
}
