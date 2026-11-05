//! Expressions and conditions: columns, bound values, operators, predicates,
//! function calls, casts, collations, subscripts, `CASE` and subqueries, each
//! through the pgorm-query builder method of the same meaning.

use neon::prelude::*;
use pgorm::pgorm_query::{
    BinOper, ColumnRef, Condition, Expr, ExprTrait, Func, FunctionCall, Name, NullOrdering, Order,
    SimpleExpr, SubQueryStatement,
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

/// The functions `call` reaches, each through its pgorm-query constructor
/// and taking the argument counts the constructor does.
fn function(name: &str, args: &[SimpleExpr]) -> Option<FunctionCall> {
    Some(match (name, args) {
        ("lower", [value]) => Func::lower(value.clone()),
        ("upper", [value]) => Func::upper(value.clone()),
        ("abs", [value]) => Func::abs(value.clone()),
        ("char_length", [value]) => Func::char_length(value.clone()),
        ("count", [value]) => Func::count(value.clone()),
        ("count_distinct", [value]) => Func::count_distinct(value.clone()),
        ("sum", [value]) => Func::sum(value.clone()),
        ("avg", [value]) => Func::avg(value.clone()),
        ("min", [value]) => Func::min(value.clone()),
        ("max", [value]) => Func::max(value.clone()),
        ("round", [value]) => Func::round(value.clone()),
        ("round", [value, precision]) => {
            Func::round_with_precision(value.clone(), precision.clone())
        }
        ("coalesce", [_, ..]) => Func::coalesce(args.to_vec()),
        ("random", []) => Func::random(),
        ("gen_random_uuid", []) => Func::gen_random_uuid(),
        ("uuidv4", []) => Func::uuidv4(),
        ("uuidv7", []) => Func::uuidv7(),
        ("uuidv7", [shift]) => Func::uuidv7_shifted(shift.clone()),
        ("uuid_extract_timestamp", [value]) => Func::uuid_extract_timestamp(value.clone()),
        ("uuid_extract_version", [value]) => Func::uuid_extract_version(value.clone()),
        ("generate_series", [start, stop]) => Func::generate_series(start.clone(), stop.clone()),
        ("generate_series", [start, stop, step]) => {
            Func::generate_series_step(start.clone(), stop.clone(), step.clone())
        }
        ("unnest", [array]) => Func::unnest(array.clone()),
        ("json_to_record", [json]) => Func::json_to_record(json.clone()),
        ("jsonb_to_record", [jsonb]) => Func::jsonb_to_record(jsonb.clone()),
        ("json_to_recordset", [json]) => Func::json_to_recordset(json.clone()),
        ("jsonb_to_recordset", [jsonb]) => Func::jsonb_to_recordset(jsonb.clone()),
        _ => return None,
    })
}

/// `exprCall(name, args)`: a call to one of the functions pgorm-query has a
/// constructor for. Another name or argument count is refused: the name
/// selects a constructor and never reaches SQL.
// [spec:pgorm:req:bindings.expressions+4]
fn call(cx: &mut FunctionContext) -> NeonResult<Node> {
    let value = arg(cx, 0);
    let name = crate::values::read::text(cx, value, "a function's name")?;
    let args = operands_at(cx, 1)?;
    match function(&name, &args) {
        Some(call) => Ok(Node::Expr(call.into())),
        None => refuse(
            cx,
            format!(
                "call() has no {name:?} taking {} argument(s); it reaches lower, upper, abs, \
                 char_length, count, count_distinct, sum, avg, min, max, round, coalesce, random, \
                 gen_random_uuid, uuidv4, uuidv7, uuid_extract_timestamp, \
                 uuid_extract_version, generate_series, unnest, json_to_record, \
                 jsonb_to_record, json_to_recordset and jsonb_to_recordset",
                args.len()
            ),
        ),
    }
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
// [spec:pgorm:req:bindings.quantifiers+3]
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
// [spec:pgorm:req:bindings.quantifiers+3]
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
// [spec:pgorm:req:bindings.quantifiers+3]
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
// [spec:pgorm:req:bindings.quantifiers+3]
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
