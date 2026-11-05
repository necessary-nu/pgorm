//! How an expression is written as an operand of a comparison: a column
//! whose domain is over an enum cast to the enum, and a literal or bound value
//! compared with an entity column cast as that column's `ColumnTrait` casts a
//! compared value.

use std::borrow::Cow;

use pgorm_query::TypeName;

use super::adapter::{self, BinOp, PlExpr};
use super::expr::{Expr, branded};
use super::sources::{pinned_cast, typed_cast};

/// What an expression is as the value side of a comparison, which decides
/// whether a column's `values_as` cast applies to it.
// [spec:pgorm:sem:entity.traits.column.enum-cast+17]
#[derive(Debug, Clone)]
pub(super) enum Operand {
    /// An expression the pipeline writes as it is: a column, a name, an
    /// operator's result, `null`.
    Written,
    /// A Rust literal, inlined, which a cast reads as the literal it is.
    Literal,
    /// A bound placeholder, with the type its value binds as where one is
    /// named (`Value::source_type_name`): the server infers a cast operand's
    /// type from the cast's target, so the placeholder is pinned to its own,
    /// as `sql.render.cast-param-type` pins a statement's.
    Bound(Option<Cow<'static, str>>),
    /// A bound built-in range or multirange, or an array of either, with the
    /// type it binds as. Cast to a column's type it goes through its text,
    /// the one form every range type and its array read: PostgreSQL has no
    /// cast from a built-in range, nor its array, to a range type a schema
    /// created (`42846`), so `ExprTrait::as_range` writes the value as its text,
    /// and the pipeline, which mints the placeholder before the comparison
    /// that casts it, has the server write it.
    // [spec:pgorm:def:sql.value.created-range+9]
    BoundRange(Cow<'static, str>),
}

pub(super) fn operand<'brand>(node: PlExpr, operand: Operand) -> Expr<'brand> {
    Expr {
        operand,
        ..branded(node)
    }
}

impl Expr<'_> {
    /// The node an operand of a comparison or an extremum is written as: cast
    /// to the enum when it is a column whose domain is over one, PostgreSQL
    /// resolving no operator or aggregate for the domain itself.
    // [spec:pgorm:sem:entity.traits.column.enum-cast+17]
    pub(super) fn compared(self) -> PlExpr {
        match &self.compares_as {
            Some(enum_type) => typed_cast(self.node, enum_type),
            None => self.node,
        }
    }

    /// The node this is written as when compared with a column whose values
    /// are cast to `values_as`: a literal or a bound placeholder cast to it,
    /// as the column's `ColumnTrait` writes a compared value, and anything
    /// else as [`compared`](Self::compared) writes it.
    // [spec:pgorm:sem:entity.traits.column.enum-cast+17]
    pub(super) fn compared_with(self, values_as: Option<&TypeName>) -> PlExpr {
        match (self.operand, values_as) {
            (Operand::Literal, Some(cast)) => typed_cast(self.node, cast),
            (Operand::Bound(pin), Some(cast)) => pinned_cast(self.node, pin.as_deref(), cast),
            (Operand::BoundRange(pin), Some(cast)) => {
                pinned_cast(self.node, Some(&format!("{pin}::text")), cast)
            }
            (operand, _) => Expr { operand, ..self }.compared(),
        }
    }
}

/// A binary operator's application. Each side that is a column whose domain
/// is over an enum is cast to the enum ([`Expr::compared`]): the comparisons
/// and `coalesce` need it, and no other operator applies to an enum at all. A
/// comparison or `coalesce` also casts a literal or bound value on either
/// side as the entity column on the other casts a value compared with it
/// ([`Expr::compared_with`]); arithmetic and logic keep their operands as they
/// are, `money * 2` being an operator of its own that `money * money` is not.
// [spec:pgorm:sem:entity.traits.column.enum-cast+17]
pub(super) fn bin<'brand>(lhs: Expr<'brand>, op: BinOp, rhs: Expr<'brand>) -> Expr<'brand> {
    let (lhs_values, rhs_values) = match op {
        BinOp::Eq
        | BinOp::Ne
        | BinOp::Gt
        | BinOp::Gte
        | BinOp::Lt
        | BinOp::Lte
        | BinOp::Coalesce => (lhs.values_as.clone(), rhs.values_as.clone()),
        _ => (None, None),
    };
    branded(adapter::binary(
        lhs.compared_with(rhs_values.as_ref()),
        op,
        rhs.compared_with(lhs_values.as_ref()),
    ))
}
