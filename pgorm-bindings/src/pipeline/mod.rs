//! A pipeline expression as a binding holds it: an owned recipe, lowered
//! into `pgorm::pipeline::Expr` only as a stage takes it.
//!
//! pgorm brands an expression holding a placeholder with its binder's
//! lifetime, which no value of either language can carry, so a binding keeps
//! the instructions instead and lowers them inside the closure pgorm hands
//! the binder to. A value with no placeholder yet is bound there too, by the
//! stage that takes it: a value never reaches the SQL as a literal unless
//! `literal()` says so.

mod stage;

use std::sync::Arc;

pub use stage::{BOUND_LIST, Plan, Stage, aggregate, filter, group, join, list};

use pgorm::{
    TableColumn,
    pgorm_query::{ColumnType, Name, TypeName, Value},
    pipeline::{self as pl, ExprOps},
};

use crate::entity::PipelineColumn;

/// The binary operators pgorm's pipeline has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Binary {
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

/// The unary operators, a sort key's direction among them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Unary {
    Not,
    Neg,
    Asc,
    Desc,
    IsNull,
    IsNotNull,
}

impl Unary {
    fn lower(self, inner: pl::Expr<'_>) -> pl::Expr<'_> {
        match self {
            Self::Not => !inner,
            Self::Neg => -inner,
            Self::Asc => inner.asc(),
            Self::Desc => inner.desc(),
            Self::IsNull => inner.is_null(),
            Self::IsNotNull => inner.is_not_null(),
        }
    }
}

/// The aggregate and window functions over one expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Function {
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
    /// The row the offset before this one in the window.
    Lag(i64),
    /// The row the offset after this one in the window.
    Lead(i64),
}

impl Function {
    fn lower(self, inner: pl::Expr<'_>) -> pl::Expr<'_> {
        match self {
            Self::Sum => pl::sum(inner),
            Self::Min => pl::min(inner),
            Self::Max => pl::max(inner),
            Self::Average => pl::average(inner),
            Self::Stddev => pl::stddev(inner),
            Self::Count => pl::count(inner),
            Self::CountDistinct => pl::count_distinct(inner),
            Self::Rank => pl::rank(inner),
            Self::RankDense => pl::rank_dense(inner),
            Self::First => pl::first(inner),
            Self::Last => pl::last(inner),
            Self::Lag(offset) => pl::lag(offset, inner),
            Self::Lead(offset) => pl::lead(offset, inner),
        }
    }
}

/// A literal written into the SQL as pgorm's pipeline writes one.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub enum Literal {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
}

/// One expression's instructions.
#[derive(Debug, Clone)]
pub enum Recipe {
    /// A literal, written into the SQL.
    Literal(Literal),
    /// A value bound by whichever stage lowers the expression.
    Value(Value),
    /// The placeholder a binder minted, by its index in the binder's values.
    Bound(usize),
    /// A column under the qualifier its relation is read by.
    Column(Name, Name),
    /// A declared model's column, under the qualifier its relation is read
    /// by, typed by its declaration.
    Typed(Name, Name, ColumnType),
    /// A declared model's column declaring casts of its own, under the name
    /// its relation is read by, compared through them.
    Shaped(Name, Name, Box<TableColumn>),
    /// A registered entity's column, under the qualifier its relation is
    /// read by.
    Registered(Name, Arc<dyn PipelineColumn>),
    /// A name a stage introduced.
    Alias(Name),
    /// `this.name`, the current relation's column.
    This(Name),
    /// `that.name`, a join's other relation's column.
    That(Name),
    #[allow(missing_docs)]
    Binary(Binary, Arc<Self>, Arc<Self>),
    #[allow(missing_docs)]
    Unary(Unary, Arc<Self>),
    /// The expression under a name of its own.
    Named(Arc<Self>, Name),
    #[allow(missing_docs)]
    Cast(Arc<Self>, pl::CastType),
    /// The expression compared as the enum type named.
    ComparesAs(Arc<Self>, TypeName),
    /// Membership in a list of expressions.
    In(Arc<Self>, Vec<Arc<Self>>),
    /// `count_rows()`.
    CountRows,
    /// `row_number()`.
    RowNumber,
    #[allow(missing_docs)]
    Function(Function, Arc<Self>),
    /// `CASE` of its arms, and the value no arm matched.
    Case(Vec<(Arc<Self>, Arc<Self>)>, Arc<Self>),
}

/// What lowering a recipe binds with: the stage's binder and the
/// placeholders its binder function minted, or neither for a stage that
/// binds nothing.
#[allow(missing_debug_implementations)]
pub struct Lowering<'a, 'brand> {
    /// The stage's binder, if it binds.
    pub binder: Option<&'a mut pl::Binder<'brand>>,
    /// The placeholders its binder function minted.
    pub bound: &'a [pl::Expr<'brand>],
}

impl Recipe {
    /// Whether lowering the recipe binds anything: a value of its own, or a
    /// binder's placeholder.
    pub fn binds(&self) -> bool {
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
            | Self::Typed(..)
            | Self::Shaped(..)
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
    pub fn lower<'brand>(&self, with: &mut Lowering<'_, 'brand>) -> Option<pl::Expr<'brand>> {
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
            Self::Typed(table, column, kind) => pl::col_typed(table.clone(), column.clone(), kind),
            Self::Shaped(table, column, casts) => {
                pl::col_shaped(table.clone(), column.clone(), casts)
            }
            Self::Registered(table, column) => column.lower(table.clone()),
            Self::Alias(name) => name.clone().into(),
            Self::This(name) => pl::this(name.clone()),
            Self::That(name) => pl::that(name.clone()),
            Self::Binary(operator, left, right) => {
                operator.lower(left.lower(with)?, right.lower(with)?)
            }
            Self::Unary(operator, inner) => operator.lower(inner.lower(with)?),
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
            Self::Function(function, inner) => function.lower(inner.lower(with)?),
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
