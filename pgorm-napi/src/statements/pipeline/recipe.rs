//! A pipeline expression as JavaScript holds it: an owned recipe, lowered
//! into `pgorm::pipeline::Expr` only as a stage takes it.
//!
//! pgorm brands an expression holding a placeholder with its binder's
//! lifetime, which no JavaScript object can carry, so the binding keeps the
//! instructions instead and lowers them inside the closure pgorm hands the
//! binder to. A value with no placeholder yet is bound there too, by the
//! stage that takes it: a value never reaches the SQL as a literal unless
//! `literal()` says so.

use std::sync::Arc;

use pgorm::{
    pgorm_query::{Name, TypeName, Value},
    pipeline::{self as pl, ExprOps},
};

use crate::entities::PipelineColumn;

/// The binary operators pgorm's pipeline has.
#[derive(Debug, Clone, Copy)]
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

pub(super) const BINARY: &[(&str, Binary)] = &[
    ("eq", Binary::Eq),
    ("ne", Binary::Ne),
    ("gt", Binary::Gt),
    ("gte", Binary::Gte),
    ("lt", Binary::Lt),
    ("lte", Binary::Lte),
    ("and", Binary::And),
    ("or", Binary::Or),
    ("coalesce", Binary::Coalesce),
    ("add", Binary::Add),
    ("sub", Binary::Sub),
    ("mul", Binary::Mul),
    ("div", Binary::Div),
    ("rem", Binary::Rem),
];

/// The unary operators.
#[derive(Debug, Clone, Copy)]
pub(super) enum Unary {
    Not,
    Neg,
    Asc,
    Desc,
    IsNull,
    IsNotNull,
}

pub(super) const UNARY: &[(&str, Unary)] = &[
    ("not", Unary::Not),
    ("neg", Unary::Neg),
    ("asc", Unary::Asc),
    ("desc", Unary::Desc),
    ("isNull", Unary::IsNull),
    ("isNotNull", Unary::IsNotNull),
];

/// The aggregate and window functions over one expression.
#[derive(Debug, Clone, Copy)]
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

pub(super) const FUNCTIONS: &[(&str, Function)] = &[
    ("sum", Function::Sum),
    ("min", Function::Min),
    ("max", Function::Max),
    ("average", Function::Average),
    ("stddev", Function::Stddev),
    ("count", Function::Count),
    ("countDistinct", Function::CountDistinct),
    ("rank", Function::Rank),
    ("rankDense", Function::RankDense),
    ("first", Function::First),
    ("last", Function::Last),
];

/// A literal written into the SQL as pgorm's pipeline writes one.
#[derive(Debug, Clone)]
pub(super) enum Literal {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
}

/// One expression's instructions.
#[derive(Debug, Clone)]
pub(super) enum Recipe {
    Literal(Literal),
    /// A value bound by whichever stage lowers the expression.
    Value(Value),
    /// The placeholder a binder minted, by its index in the binder's values.
    Bound(usize),
    Column(Name, Name),
    /// A registered entity's column, under the name its relation is read by.
    Registered(Name, Arc<dyn PipelineColumn>),
    Alias(Name),
    This(Name),
    That(Name),
    Binary(Binary, Arc<Self>, Arc<Self>),
    Unary(Unary, Arc<Self>),
    Named(Arc<Self>, Name),
    Cast(Arc<Self>, pl::CastType),
    ComparesAs(Arc<Self>, TypeName),
    In(Arc<Self>, Vec<Arc<Self>>),
    CountRows,
    RowNumber,
    Function(Function, Arc<Self>),
    Case(Vec<(Arc<Self>, Arc<Self>)>, Arc<Self>),
}

/// What lowering a recipe binds with: the stage's binder and the
/// placeholders its binder function minted, or neither for a stage that
/// binds nothing.
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
            _ => false,
        }
    }

    /// The pgorm expression, or `None` where a value or placeholder needs a
    /// binder the lowering does not have.
    // [spec:pgorm:req:bindings.pipeline-expressions+3]
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
            Self::Column(table, column) => pl::col(table.clone(), column.clone()),
            Self::Registered(table, column) => column.lower(table.clone()),
            Self::Alias(name) => name.clone().into(),
            Self::This(name) => pl::this(name.clone()),
            Self::That(name) => pl::that(name.clone()),
            Self::Binary(operator, left, right) => {
                binary(*operator, left.lower(with)?, right.lower(with)?)
            }
            Self::Unary(operator, inner) => unary(*operator, inner.lower(with)?),
            Self::Named(inner, name) => inner.lower(with)?.as_runtime(name.clone()),
            Self::Cast(inner, kind) => inner.lower(with)?.cast(*kind),
            Self::ComparesAs(inner, enum_type) => inner.lower(with)?.compares_as(enum_type.clone()),
            Self::In(inner, items) => {
                let inner = inner.lower(with)?;
                let items: Option<Vec<_>> = items.iter().map(|item| item.lower(with)).collect();
                inner.in_array(items?)
            }
            Self::CountRows => pl::count_rows(),
            Self::RowNumber => pl::row_number(),
            Self::Function(function, inner) => call(*function, inner.lower(with)?),
            Self::Case(arms, otherwise) => {
                let mut lowered = Vec::with_capacity(arms.len());
                for (condition, value) in arms {
                    lowered.push((condition.lower(with)?, value.lower(with)?));
                }
                pl::case(lowered, otherwise.lower(with)?)
            }
        })
    }
}

fn binary<'brand>(
    operator: Binary,
    left: pl::Expr<'brand>,
    right: pl::Expr<'brand>,
) -> pl::Expr<'brand> {
    match operator {
        Binary::Eq => left.eq(right),
        Binary::Ne => left.ne(right),
        Binary::Gt => left.gt(right),
        Binary::Gte => left.gte(right),
        Binary::Lt => left.lt(right),
        Binary::Lte => left.lte(right),
        Binary::And => left.and(right),
        Binary::Or => left.or(right),
        Binary::Coalesce => left.coalesce(right),
        Binary::Add => left.add(right),
        Binary::Sub => left.sub(right),
        Binary::Mul => left.mul(right),
        Binary::Div => left.div(right),
        Binary::Rem => left.rem(right),
    }
}

fn unary(operator: Unary, inner: pl::Expr<'_>) -> pl::Expr<'_> {
    match operator {
        Unary::Not => !inner,
        Unary::Neg => -inner,
        Unary::Asc => inner.asc(),
        Unary::Desc => inner.desc(),
        Unary::IsNull => inner.is_null(),
        Unary::IsNotNull => inner.is_not_null(),
    }
}

fn call(function: Function, inner: pl::Expr<'_>) -> pl::Expr<'_> {
    match function {
        Function::Sum => pl::sum(inner),
        Function::Min => pl::min(inner),
        Function::Max => pl::max(inner),
        Function::Average => pl::average(inner),
        Function::Stddev => pl::stddev(inner),
        Function::Count => pl::count(inner),
        Function::CountDistinct => pl::count_distinct(inner),
        Function::Rank => pl::rank(inner),
        Function::RankDense => pl::rank_dense(inner),
        Function::First => pl::first(inner),
        Function::Last => pl::last(inner),
        Function::Lag(offset) => pl::lag(offset, inner),
        Function::Lead(offset) => pl::lead(offset, inner),
    }
}
