//! Reading a column's `DEFAULT` or generation expression out of
//! PostgreSQL's parse tree into the subset `ColumnExpr` holds, naming
//! everything else.

use super::unsupported;
use crate::{CaseExpr, CastType, ColumnExpr, CurrentKeyword, Error, Operator};
use pg_query::NodeEnum;
use pg_query::protobuf::{
    AConst, AExpr, AExprKind, BoolExpr, BoolExprType, CaseExpr as PgCaseExpr, CoercionForm,
    FuncCall, MinMaxOp, Node, NullTest, NullTestType, SqlValueFunction, SqlValueFunctionOp,
    TypeCast, TypeName, a_const,
};

/// The schema holding PostgreSQL's built-in functions and types, which the
/// server searches before the schemas `search_path` names unless the path
/// places it later, so an expression names them bare.
const CATALOG: &str = "pg_catalog";

/// The calls PostgreSQL's grammar builds itself, which pgorm writes bare.
const GRAMMAR_CALLS: [&str; 4] = ["coalesce", "greatest", "least", "nullif"];

/// Read `node`, the expression of `what` (`the DEFAULT of column `t`.`c``),
/// into the subset, or name the first construct outside it.
// [spec:pgorm:sem:codegen.ddl.tables+11]
pub(super) fn read(node: &Node, what: &str, at: usize) -> Result<ColumnExpr, Error> {
    let refuse = |construct: &str| unsupported(format!("{construct} in {what}"), at);
    let Some(node) = &node.node else {
        return Err(refuse("an empty expression"));
    };
    match node {
        NodeEnum::AConst(constant) => constant_expr(constant).ok_or_else(|| refuse("a literal")),
        NodeEnum::ColumnRef(column) => match names(&column.fields).as_deref() {
            Some([name]) => Ok(ColumnExpr::Column(name.clone())),
            _ => Err(refuse("a qualified column reference")),
        },
        NodeEnum::AExpr(expr) => operator(expr, what, at),
        NodeEnum::BoolExpr(expr) => boolean(expr, what, at),
        NodeEnum::NullTest(test) => null_test(test, what, at),
        NodeEnum::FuncCall(call) => function(call, what, at),
        // The grammar builds these four calls itself, and pgorm writes them
        // bare, as calls by name.
        NodeEnum::CoalesceExpr(call) => call_of("coalesce", &call.args, what, at),
        NodeEnum::MinMaxExpr(call) => match MinMaxOp::try_from(call.op) {
            Ok(MinMaxOp::IsGreatest) => call_of("greatest", &call.args, what, at),
            Ok(MinMaxOp::IsLeast) => call_of("least", &call.args, what, at),
            _ => Err(refuse("an unrecognised GREATEST or LEAST")),
        },
        NodeEnum::TypeCast(cast) => type_cast(cast, what, at),
        NodeEnum::CaseExpr(case) => case_expr(case, what, at),
        NodeEnum::SqlvalueFunction(function) => keyword(function).ok_or_else(|| {
            refuse(
                "a SQL value function other than CURRENT_DATE, CURRENT_TIME or CURRENT_TIMESTAMP",
            )
        }),
        _ => Err(refuse("an expression codegen does not read")),
    }
}

fn read_boxed(node: &Option<Box<Node>>, what: &str, at: usize) -> Result<ColumnExpr, Error> {
    match node {
        Some(node) => read(node, what, at),
        None => Err(unsupported(format!("a missing operand in {what}"), at)),
    }
}

/// The string parts of a dotted name, or `None` when a part is not a name.
fn names(nodes: &[Node]) -> Option<Vec<String>> {
    nodes
        .iter()
        .map(|node| match &node.node {
            Some(NodeEnum::String(part)) => Some(part.sval.clone()),
            _ => None,
        })
        .collect()
}

/// A dotted name with `pg_catalog`'s qualification dropped: the schema and
/// the name.
fn catalog_name(nodes: &[Node]) -> Option<(Option<String>, String)> {
    match names(nodes)?.as_slice() {
        [name] => Some((None, name.clone())),
        [schema, name] if schema == CATALOG => Some((None, name.clone())),
        [schema, name] => Some((Some(schema.clone()), name.clone())),
        _ => None,
    }
}

fn constant_expr(constant: &AConst) -> Option<ColumnExpr> {
    if constant.isnull {
        return Some(ColumnExpr::Null);
    }
    match constant.val.as_ref()? {
        a_const::Val::Ival(int) => Some(ColumnExpr::Int(int.ival)),
        a_const::Val::Fval(float) => ColumnExpr::numeric(&float.fval),
        a_const::Val::Boolval(value) => Some(ColumnExpr::Bool(value.boolval)),
        a_const::Val::Sval(text) => Some(ColumnExpr::Text(text.sval.clone())),
        a_const::Val::Bsval(_) => None,
    }
}

fn call_of(name: &str, args: &[Node], what: &str, at: usize) -> Result<ColumnExpr, Error> {
    let args = args
        .iter()
        .map(|arg| read(arg, what, at))
        .collect::<Result<_, _>>()?;
    Ok(ColumnExpr::Call(name.to_owned(), args))
}

fn operator(expr: &AExpr, what: &str, at: usize) -> Result<ColumnExpr, Error> {
    if AExprKind::try_from(expr.kind) == Ok(AExprKind::AexprNullif) {
        let operands = [&expr.lexpr, &expr.rexpr].map(|operand| read_boxed(operand, what, at));
        let [left, right] = operands;
        return Ok(ColumnExpr::Call("nullif".to_owned(), vec![left?, right?]));
    }
    let op = match (AExprKind::try_from(expr.kind), names(&expr.name).as_deref()) {
        (Ok(AExprKind::AexprOp), Some([op])) => Operator::from_sql(op),
        _ => None,
    };
    match (op, &expr.lexpr) {
        (Some(op), Some(_)) => Ok(ColumnExpr::Binary(
            Box::new(read_boxed(&expr.lexpr, what, at)?),
            op,
            Box::new(read_boxed(&expr.rexpr, what, at)?),
        )),
        _ => Err(unsupported(
            format!("an operator codegen does not read in {what}"),
            at,
        )),
    }
}

fn boolean(expr: &BoolExpr, what: &str, at: usize) -> Result<ColumnExpr, Error> {
    let mut args = expr.args.iter().map(|arg| read(arg, what, at));
    let op = match BoolExprType::try_from(expr.boolop) {
        Ok(BoolExprType::NotExpr) => {
            let operand = args.next().unwrap_or_else(|| {
                Err(unsupported(
                    format!("a NOT without an operand in {what}"),
                    at,
                ))
            })?;
            return Ok(ColumnExpr::Not(Box::new(operand)));
        }
        Ok(BoolExprType::AndExpr) => Operator::And,
        Ok(BoolExprType::OrExpr) => Operator::Or,
        _ => return Err(unsupported(format!("a boolean operator in {what}"), at)),
    };
    let first = args.next().unwrap_or_else(|| {
        Err(unsupported(
            format!("a boolean operator without operands in {what}"),
            at,
        ))
    })?;
    args.try_fold(first, |left, right| {
        Ok(ColumnExpr::Binary(Box::new(left), op, Box::new(right?)))
    })
}

fn null_test(test: &NullTest, what: &str, at: usize) -> Result<ColumnExpr, Error> {
    let negated = match NullTestType::try_from(test.nulltesttype) {
        Ok(NullTestType::IsNull) => false,
        Ok(NullTestType::IsNotNull) => true,
        _ => return Err(unsupported(format!("a null test in {what}"), at)),
    };
    if test.argisrow {
        return Err(unsupported(format!("a row null test in {what}"), at));
    }
    Ok(ColumnExpr::IsNull(
        Box::new(read_boxed(&test.arg, what, at)?),
        negated,
    ))
}

fn function(call: &FuncCall, what: &str, at: usize) -> Result<ColumnExpr, Error> {
    let refuse = |construct: &str| unsupported(format!("{construct} in {what}"), at);
    let decorated = !call.agg_order.is_empty()
        || call.agg_filter.is_some()
        || call.over.is_some()
        || call.agg_within_group
        || call.agg_star
        || call.agg_distinct
        || call.func_variadic;
    if decorated {
        return Err(refuse("an aggregate, window or variadic call"));
    }
    if CoercionForm::try_from(call.funcformat) != Ok(CoercionForm::CoerceExplicitCall) {
        return Err(refuse("a function written in SQL syntax"));
    }
    let name = match catalog_name(&call.funcname) {
        Some((None, name)) => name,
        _ => return Err(refuse("a schema-qualified function")),
    };
    // Called by name, these four are the grammar's constructs: a function
    // the parse tree names so was quoted, and is some other function.
    if GRAMMAR_CALLS.contains(&name.as_str()) {
        return Err(refuse(&format!("a function quoted to be named `{name}`")));
    }
    call_of(&name, &call.args, what, at)
}

/// A searched `CASE`. The simple form, `CASE <operand> WHEN <value> ..`,
/// compares one operand with each arm's value, which the subset's arms of
/// conditions do not hold, so it is named.
fn case_expr(case: &PgCaseExpr, what: &str, at: usize) -> Result<ColumnExpr, Error> {
    let refuse = |construct: &str| unsupported(format!("{construct} in {what}"), at);
    if case.arg.is_some() {
        return Err(refuse("a CASE comparing an operand"));
    }
    let mut arms = case.args.iter().map(|arm| match &arm.node {
        Some(NodeEnum::CaseWhen(arm)) => Ok((
            read_boxed(&arm.expr, what, at)?,
            read_boxed(&arm.result, what, at)?,
        )),
        _ => Err(refuse("a CASE arm codegen does not read")),
    });
    let first = arms
        .next()
        .unwrap_or_else(|| Err(refuse("a CASE with no WHEN")))?;
    Ok(ColumnExpr::Case(Box::new(CaseExpr {
        first,
        rest: arms.collect::<Result<_, _>>()?,
        otherwise: case
            .defresult
            .as_ref()
            .map(|otherwise| read(otherwise, what, at))
            .transpose()?,
    })))
}

fn type_cast(cast: &TypeCast, what: &str, at: usize) -> Result<ColumnExpr, Error> {
    let refuse = |construct: &str| unsupported(format!("{construct} in {what}"), at);
    let Some(type_name) = &cast.type_name else {
        return Err(refuse("a cast to no type"));
    };
    let cast_type = cast_type(type_name).ok_or_else(|| refuse("a cast to a modified type"))?;
    Ok(ColumnExpr::Cast(
        Box::new(read_boxed(&cast.arg, what, at)?),
        cast_type,
    ))
}

/// The type a cast names, when it is a plain name: no modifier, no `SETOF`
/// or `%TYPE`, and at most one array dimension, given without a bound.
fn cast_type(type_name: &TypeName) -> Option<CastType> {
    let array = match type_name.array_bounds.as_slice() {
        [] => false,
        [bound] => matches!(&bound.node, Some(NodeEnum::Integer(int)) if int.ival == -1),
        _ => return None,
    };
    if !type_name.typmods.is_empty() || type_name.setof || type_name.pct_type {
        return None;
    }
    let (schema, name) = catalog_name(&type_name.names)?;
    Some(CastType {
        schema,
        name,
        array,
    })
}

fn keyword(function: &SqlValueFunction) -> Option<ColumnExpr> {
    let keyword = match SqlValueFunctionOp::try_from(function.op).ok()? {
        SqlValueFunctionOp::SvfopCurrentDate => CurrentKeyword::Date,
        SqlValueFunctionOp::SvfopCurrentTime => CurrentKeyword::Time,
        SqlValueFunctionOp::SvfopCurrentTimestamp => CurrentKeyword::Timestamp,
        _ => return None,
    };
    Some(ColumnExpr::Keyword(keyword))
}
