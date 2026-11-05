//! Owned expression instructions lowered only inside their Rust binder brand.
//!
//! A value with no placeholder yet is bound by the stage that takes it, through
//! that stage's binder: a value reaches the SQL as a literal only through
//! `literal()`.

use std::sync::Arc;

use pgorm::pgorm_query::{Name, TypeName, Value};
use pgorm::pipeline::{self as pl, ExprOps};

use crate::entities::backend::PipelineColumn;

#[derive(Clone, Copy, Debug)]
pub(super) enum Binary {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    And,
    Or,
    Coalesce,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl Binary {
    fn lower<'brand>(self, left: pl::Expr<'brand>, right: pl::Expr<'brand>) -> pl::Expr<'brand> {
        match self {
            Self::Eq => left.eq(right),
            Self::Ne => left.ne(right),
            Self::Gt => left.gt(right),
            Self::Gte => left.gte(right),
            Self::Lt => left.lt(right),
            Self::Lte => left.lte(right),
            Self::And => left.and(right),
            Self::Or => left.or(right),
            Self::Coalesce => left.coalesce(right),
            Self::Add => left.add(right),
            Self::Sub => left.sub(right),
            Self::Mul => left.mul(right),
            Self::Div => left.div(right),
            Self::Rem => left.rem(right),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Unary {
    Not,
    Neg,
    Asc,
    Desc,
    IsNull,
    IsNotNull,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Function {
    Sum,
    Min,
    Max,
    Average,
    Stddev,
    Count,
    CountDistinct,
    Rank,
    RankDense,
    First,
    Last,
    Lag(i64),
    Lead(i64),
}

/// A literal written into the SQL as pgorm's pipeline writes one.
#[derive(Clone, Debug)]
pub(super) enum Literal {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
}

#[derive(Clone, Debug)]
pub(super) enum Recipe {
    Literal(Literal),
    /// A value bound by whichever stage lowers the expression.
    Value(Value),
    Column(String, String),
    /// A registered entity's column, under the qualifier its relation is read by.
    Registered(String, Arc<dyn PipelineColumn>),
    Alias(String),
    This(String),
    That(String),
    Bound(usize),
    Binary(Binary, Arc<Self>, Arc<Self>),
    Unary(Unary, Arc<Self>),
    Named(Arc<Self>, String),
    Cast(Arc<Self>, pl::CastType),
    ComparesAs(Arc<Self>, TypeName),
    In(Arc<Self>, Vec<Arc<Self>>),
    CountRows,
    RowNumber,
    Function(Function, Arc<Self>),
    Case(Vec<(Arc<Self>, Arc<Self>)>, Arc<Self>),
}

/// What lowering a recipe binds with: the stage's binder and the placeholders
/// its binder function minted, or neither for a stage that binds nothing.
pub(super) struct Lowering<'a, 'brand> {
    pub(super) binder: Option<&'a mut pl::Binder<'brand>>,
    pub(super) bound: &'a [pl::Expr<'brand>],
}

impl Recipe {
    /// Whether lowering the recipe binds anything: a value of its own, or a
    /// binder's placeholder.
    pub(super) fn binds(&self) -> bool {
        match self {
            Self::Value(_) | Self::Bound(_) => true,
            Self::Binary(_, left, right) => left.binds() || right.binds(),
            Self::Unary(_, inner)
            | Self::Named(inner, _)
            | Self::Cast(inner, _)
            | Self::ComparesAs(inner, _)
            | Self::Function(_, inner) => inner.binds(),
            Self::In(inner, items) => inner.binds() || items.iter().any(|item| item.binds()),
            Self::Case(arms, otherwise) => {
                otherwise.binds()
                    || arms
                        .iter()
                        .any(|(condition, value)| condition.binds() || value.binds())
            }
            Self::Literal(_)
            | Self::Column(..)
            | Self::Registered(..)
            | Self::Alias(_)
            | Self::This(_)
            | Self::That(_)
            | Self::CountRows
            | Self::RowNumber => false,
        }
    }

    /// The pgorm expression, or `None` where a value or placeholder needs a
    /// binder the lowering does not have.
    // [spec:pgorm:req:bindings.pipeline-expressions+3]
    // [spec:pgorm:req:bindings.operands]
    pub(super) fn lower<'brand>(
        &self,
        with: &mut Lowering<'_, 'brand>,
    ) -> Option<pl::Expr<'brand>> {
        Some(match self {
            Self::Literal(literal) => match literal {
                Literal::Null => pl::null(),
                Literal::Bool(value) => (*value).into(),
                Literal::Integer(value) => (*value).into(),
                Literal::Float(value) => (*value).into(),
                Literal::Text(value) => value.as_str().into(),
            },
            Self::Value(value) => with.binder.as_mut()?.bind(value.clone()),
            Self::Bound(index) => with.bound.get(*index)?.clone(),
            Self::Column(table, column) => pl::col(Name::runtime(table), Name::runtime(column)),
            Self::Registered(table, column) => column.lower(table),
            Self::Alias(name) => Name::runtime(name).into(),
            Self::This(name) => pl::this(Name::runtime(name)),
            Self::That(name) => pl::that(Name::runtime(name)),
            Self::Binary(operation, left, right) => {
                operation.lower(left.lower(with)?, right.lower(with)?)
            }
            Self::Unary(operation, value) => {
                let value = value.lower(with)?;
                match operation {
                    Unary::Not => !value,
                    Unary::Neg => -value,
                    Unary::Asc => value.asc(),
                    Unary::Desc => value.desc(),
                    Unary::IsNull => value.is_null(),
                    Unary::IsNotNull => value.is_not_null(),
                }
            }
            Self::Named(value, name) => value.lower(with)?.as_runtime(Name::runtime(name)),
            Self::Cast(value, kind) => value.lower(with)?.cast(*kind),
            Self::ComparesAs(value, enum_type) => value.lower(with)?.compares_as(enum_type.clone()),
            Self::In(value, members) => {
                let value = value.lower(with)?;
                let members = members
                    .iter()
                    .map(|member| member.lower(with))
                    .collect::<Option<Vec<_>>>()?;
                value.in_array(members)
            }
            Self::CountRows => pl::count_rows(),
            Self::RowNumber => pl::row_number(),
            Self::Function(function, argument) => {
                let argument = argument.lower(with)?;
                match function {
                    Function::Sum => pl::sum(argument),
                    Function::Min => pl::min(argument),
                    Function::Max => pl::max(argument),
                    Function::Average => pl::average(argument),
                    Function::Stddev => pl::stddev(argument),
                    Function::Count => pl::count(argument),
                    Function::CountDistinct => pl::count_distinct(argument),
                    Function::Rank => pl::rank(argument),
                    Function::RankDense => pl::rank_dense(argument),
                    Function::First => pl::first(argument),
                    Function::Last => pl::last(argument),
                    Function::Lag(offset) => pl::lag(*offset, argument),
                    Function::Lead(offset) => pl::lead(*offset, argument),
                }
            }
            Self::Case(arms, otherwise) => {
                let mut lowered = Vec::with_capacity(arms.len());
                for (when, then) in arms {
                    lowered.push((when.lower(with)?, then.lower(with)?));
                }
                pl::case(lowered, otherwise.lower(with)?)
            }
        })
    }
}
