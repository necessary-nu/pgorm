use pgorm::pgorm_query::{
    BinOper, ColumnRef, Condition, Expr, ExprTrait, IntoCondition, OrderedStatement, Query,
    ReturningClause, SelectStatement, SimpleExpr, Values,
};
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyInt, PyTuple},
};

use crate::{
    errors::ConstructionError,
    expressions::{Aliased, Compiled, OrderBy, PyCondition, require_expr},
    identifiers::Identifier,
};

pub(crate) fn condition(value: &Bound<'_, PyAny>) -> PyResult<Condition> {
    if let Ok(condition) = value.extract::<PyRef<'_, PyCondition>>() {
        Ok(condition.inner.clone())
    } else {
        Ok(require_expr(value)?.inner.into_condition())
    }
}

/// The columns `items` names, each a column expression as pgorm-query's
/// `IntoColumnRef` takes one, and at least one: `what` names the method in
/// the error refusing another expression or none.
// [spec:pgorm:req:bindings.distinct-on]
pub(super) fn columns(items: &Bound<'_, PyTuple>, what: &str) -> PyResult<Vec<ColumnRef>> {
    if items.is_empty() {
        return Err(ConstructionError::new_err(format!(
            "{what} names at least one column"
        )));
    }
    items
        .iter()
        .map(|item| match require_expr(&item)?.inner {
            SimpleExpr::Column(column) => Ok(column),
            _ => Err(ConstructionError::new_err(format!(
                "{what} takes columns, which an expression is not"
            ))),
        })
        .collect()
}

pub(super) fn project(query: &mut SelectStatement, items: &Bound<'_, PyTuple>) -> PyResult<()> {
    for item in items.iter() {
        if let Ok(item) = item.extract::<PyRef<'_, Aliased>>() {
            query.expr_as(item.expr.inner.clone(), item.alias.name());
        } else if let Ok(item) = item.extract::<PyRef<'_, super::window::PyWindowed>>() {
            item.project(query);
        } else {
            query.expr(require_expr(&item)?.inner);
        }
    }
    Ok(())
}

/// Append each `OrderBy` to a statement's or a window's ORDER BY.
pub(super) fn order<S: OrderedStatement>(
    statement: &mut S,
    orderings: &Bound<'_, PyTuple>,
) -> PyResult<()> {
    for ordering in orderings.iter() {
        let ordering = ordering
            .extract::<PyRef<'_, OrderBy>>()
            .map_err(|_| PyTypeError::new_err("order_by requires OrderBy expressions"))?;
        match &ordering.nulls {
            Some(nulls) => statement.order_by_expr_with_nulls(
                ordering.expr.inner.clone(),
                ordering.direction.rust_order(),
                nulls.rust_nulls(),
            ),
            None => statement
                .order_by_expr(ordering.expr.inner.clone(), ordering.direction.rust_order()),
        };
    }
    Ok(())
}

pub(super) fn unsigned(value: &Bound<'_, PyAny>) -> PyResult<u64> {
    if !value.is_exact_instance_of::<PyInt>() {
        return Err(PyTypeError::new_err("limit/offset requires an integer"));
    }
    let value: u64 = value
        .extract()
        .map_err(|_| ConstructionError::new_err("limit/offset must be non-negative and fit u64"))?;
    if value > i64::MAX as u64 {
        return Err(ConstructionError::new_err(
            "limit/offset exceeds PostgreSQL's signed bigint range",
        ));
    }
    Ok(value)
}

pub(super) fn compiled((sql, values): (String, Values)) -> PyResult<Compiled> {
    if values.0.len() > 65535 {
        return Err(ConstructionError::new_err(
            "statement exceeds PostgreSQL's 65535-parameter limit",
        ));
    }
    Ok(Compiled { sql, values })
}

/// A RETURNING list, `*` when empty, with the written row's versions renamed
/// when `old_as` or `new_as` names them.
pub(super) fn returning(
    items: &Bound<'_, PyTuple>,
    old_as: Option<&Bound<'_, PyAny>>,
    new_as: Option<&Bound<'_, PyAny>>,
) -> PyResult<ReturningClause> {
    let mut clause = list(items)?;
    if let Some(name) = old_as {
        clause = clause.old_as(Identifier::new(name)?.name());
    }
    if let Some(name) = new_as {
        clause = clause.new_as(Identifier::new(name)?.name());
    }
    Ok(clause)
}

fn list(items: &Bound<'_, PyTuple>) -> PyResult<ReturningClause> {
    if items.is_empty() {
        return Ok(Query::returning().all());
    }
    let items = items
        .iter()
        .map(|item| {
            if let Ok(item) = item.extract::<PyRef<'_, Aliased>>() {
                Ok(item
                    .expr
                    .inner
                    .clone()
                    .binary(BinOper::As, Expr::col(item.alias.name())))
            } else {
                Ok(require_expr(&item)?.inner)
            }
        })
        .collect::<PyResult<Vec<_>>>()?;
    Ok(Query::returning().exprs(items))
}
