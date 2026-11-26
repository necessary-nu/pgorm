//! Native preparation metadata, independent of result values and aliases.
//!
//! The pinned compiler erases some PostgreSQL identities: REAL is FLOAT8;
//! rewritten SMALLINT casts become INTEGER (INT4), and VARCHAR/BPCHAR casts
//! become TEXT. UUID/JSON/JSONB casts also become TEXT, and NUMERIC casts become
//! REAL; these do not retain UUID, JSON or decimal identity. Declared
//! SMALLINT/BIGINT and length-qualified VARCHAR table columns keep their types.
//! Missing information is never recovered from an output name or a first row.

use super::codecs::ScalarCodec;
use crate::types::Type;
use crate::{Column, Error};
use std::fmt;
use turso_core::ast::{Expr, LikeOperator, Literal, Operator, SubqueryType, UnaryOperator};
use turso_core::{ColumnTypeInfo, ColumnTypeKind};

#[derive(Debug)]
struct MetadataError {
    index: usize,
    name: String,
    reason: &'static str,
    native_type: Option<String>,
}

impl fmt::Display for MetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "column {} ({:?}): {}",
            self.index + 1,
            self.name,
            self.reason
        )?;
        if let Some(ty) = &self.native_type {
            write!(f, " (native type {ty:?})")?;
        }
        Ok(())
    }
}

impl std::error::Error for MetadataError {}

// [spec:pgorm:req:turso.rows]
// [spec:pgorm:req:turso.errors]
pub(crate) fn describe_columns(
    native: &turso_pg::Statement,
    parameter_types: &[Type],
) -> Result<(Vec<Column>, Box<[ScalarCodec]>), Error> {
    let mut columns = Vec::with_capacity(native.num_columns());
    let mut codecs = Vec::with_capacity(native.num_columns());
    for index in 0..native.num_columns() {
        let name = native.get_column_name(index).into_owned();
        let info = native.get_column_type_info(index).map_err(Error::turso)?;
        let expr = native
            .get_program()
            .result_columns
            .get(index)
            .map(|column| &column.expr);
        let ty = resolve_type(expr, info.as_ref(), parameter_types).map_err(|reason| {
            Error::turso_unsupported_with(
                "result column typing",
                MetadataError {
                    index,
                    name: name.clone(),
                    reason,
                    native_type: info.as_ref().map(|info| info.declared_name.clone()),
                },
            )
        })?;
        let codec =
            ScalarCodec::for_type(&ty).expect("metadata resolver returned a supported type");
        columns.push(Column {
            name,
            table_oid: None,
            column_id: None,
            type_modifier: -1,
            r#type: ty,
        });
        codecs.push(codec);
    }
    Ok((columns, codecs.into_boxed_slice()))
}

fn resolve_type(
    expr: Option<&Expr>,
    info: Option<&ColumnTypeInfo>,
    parameter_types: &[Type],
) -> Result<Type, &'static str> {
    if info.is_some_and(|info| info.array_dimensions != 0) {
        return Err("native array result types are unsupported");
    }
    let expr = expr.ok_or("native compiler did not provide a result expression")?;
    match expr {
        Expr::Parenthesized(expressions) if expressions.len() == 1 => {
            resolve_type(Some(&expressions[0]), info, parameter_types)
        }
        Expr::Collate(inner, _) => resolve_type(Some(inner), info, parameter_types),
        Expr::Variable(variable) => parameter_types
            .get(
                usize::try_from(variable.index.get() - 1)
                    .map_err(|_| "native parameter index is too large")?,
            )
            .cloned()
            .filter(|ty| ScalarCodec::for_type(ty).is_some())
            .ok_or("native result parameter needs an explicit supported parameter type"),
        Expr::Column { .. } => declared_type(info.ok_or("native column type is unavailable")?),
        Expr::Cast {
            type_name: Some(ty),
            ..
        } if ty.array_dimensions == 0 => {
            // BLOB casts lack ColumnTypeInfo at this revision, but the native
            // compiler's explicit cast retains this identity.
            if ty.name.eq_ignore_ascii_case("BLOB") {
                Ok(Type::BYTEA)
            } else {
                declared_type(info.ok_or("native cast type is unavailable")?)
            }
        }
        Expr::FunctionCall { name, args, .. }
            if name.as_str().eq_ignore_ascii_case("count") && args.len() == 1 =>
        {
            Ok(Type::INT8)
        }
        Expr::FunctionCallStar { name, .. } if name.as_str().eq_ignore_ascii_case("count") => {
            Ok(Type::INT8)
        }
        Expr::Binary(
            _,
            Operator::And
            | Operator::Or
            | Operator::Equals
            | Operator::NotEquals
            | Operator::Greater
            | Operator::GreaterEquals
            | Operator::Less
            | Operator::LessEquals
            | Operator::Is
            | Operator::IsNot,
            _,
        ) => Ok(Type::BOOL),
        Expr::Unary(UnaryOperator::Not, _)
        | Expr::IsNull(_)
        | Expr::NotNull(_)
        | Expr::Between { .. }
        | Expr::InList { .. }
        | Expr::InSelect { .. }
        | Expr::InTable { .. }
        | Expr::Exists(_)
        | Expr::Like {
            op: LikeOperator::Like | LikeOperator::Glob,
            ..
        }
        | Expr::SubqueryResult {
            query_type: SubqueryType::Exists { .. } | SubqueryType::In { .. },
            ..
        } => Ok(Type::BOOL),
        Expr::Literal(Literal::Numeric(value)) => numeric_literal_type(value),
        Expr::Literal(Literal::String(_)) => Ok(Type::TEXT),
        Expr::Literal(Literal::Blob(_)) => Ok(Type::BYTEA),
        Expr::Literal(Literal::True | Literal::False) => Ok(Type::BOOL),
        // Affinity INTEGER alone cannot establish the width of arbitrary
        // arithmetic, aggregates, subqueries, or user-defined functions.
        _ => Err("native expression type is unresolved; use a supported explicit cast"),
    }
}

fn numeric_literal_type(value: &str) -> Result<Type, &'static str> {
    if let Ok(integer) = value.parse::<i64>() {
        return Ok(if i32::try_from(integer).is_ok() {
            Type::INT4
        } else {
            Type::INT8
        });
    }
    // PostgreSQL decimal literals have NUMERIC identity. Native REAL affinity
    // does not preserve it, so callers must request an explicit floating cast.
    Err("native numeric literal needs an explicit supported integer or floating cast")
}

fn declared_type(info: &ColumnTypeInfo) -> Result<Type, &'static str> {
    if info.array_dimensions != 0 {
        return Err("native array result types are unsupported");
    }
    let name = &info.declared_name;
    match info.kind {
        ColumnTypeKind::Builtin => {
            if name.eq_ignore_ascii_case("INTEGER") {
                Ok(Type::INT4)
            } else if name.eq_ignore_ascii_case("BOOLEAN") || name.eq_ignore_ascii_case("BOOL") {
                Ok(Type::BOOL)
            } else if name.eq_ignore_ascii_case("TEXT") {
                Ok(Type::TEXT)
            } else if name.eq_ignore_ascii_case("REAL") {
                Ok(Type::FLOAT8)
            } else if name.eq_ignore_ascii_case("BLOB") {
                Ok(Type::BYTEA)
            } else {
                Err("native builtin result type has no supported PostgreSQL scalar mapping")
            }
        }
        ColumnTypeKind::Custom => {
            // Turso registers these PostgreSQL builtins as custom core types.
            // Do not flatten arbitrary user-defined types via their base type.
            if name.eq_ignore_ascii_case("smallint") {
                Ok(Type::INT2)
            } else if name.eq_ignore_ascii_case("bigint") {
                Ok(Type::INT8)
            } else if name.eq_ignore_ascii_case("boolean") || name.eq_ignore_ascii_case("bool") {
                Ok(Type::BOOL)
            } else if name.eq_ignore_ascii_case("varchar") {
                Ok(Type::VARCHAR)
            } else if name.eq_ignore_ascii_case("bpchar") {
                Ok(Type::BPCHAR)
            } else if name.eq_ignore_ascii_case("bytea") {
                Ok(Type::BYTEA)
            } else {
                Err("native custom result type has no supported PostgreSQL scalar mapping")
            }
        }
        _ => Err("native domain, composite, union or unknown result types are unsupported"),
    }
}

#[cfg(test)]
mod tests;
