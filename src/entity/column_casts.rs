//! The casts a disjoint statement reads, writes and compares one column
//! under: a typed column's own, or those a column of its type takes by
//! default.

use std::sync::Arc;

use pgorm_query::{ColumnType, Expr, SimpleExpr, TypeName, Value};

use super::{compared_enum, compared_value_as, save_cast, select_cast};
use crate::{ColumnTrait, executor::BoundaryCast};

/// The casts one column is read, written and compared under.
trait Casts: Send + Sync {
    /// `expr`, a read of the column, as a projection reads it.
    fn select(&self, expr: Expr) -> SimpleExpr;

    /// `value` as it is written to the column.
    fn save(&self, value: Expr) -> SimpleExpr;

    /// `value` as a value predicate on the column compares it: cast to the
    /// enum when the column's domain is over one, and written through its
    /// `save_as` otherwise.
    fn compared(&self, value: Expr) -> SimpleExpr;
}

/// The casts of a typed column `C`: its own `select_as` and `save_as`,
/// overrides included.
struct Typed<C>(C);

impl<C: ColumnTrait> Casts for Typed<C> {
    fn select(&self, expr: Expr) -> SimpleExpr {
        self.0.select_as(expr)
    }

    fn save(&self, value: Expr) -> SimpleExpr {
        self.0.save_as(value)
    }

    fn compared(&self, value: Expr) -> SimpleExpr {
        super::compared_value(&self.0, value)
    }
}

/// The casts of a column of the type that declares none of its own: the
/// functions `ColumnTrait`'s defaults call.
impl Casts for ColumnType {
    fn select(&self, expr: Expr) -> SimpleExpr {
        select_cast(self, expr)
    }

    fn save(&self, value: Expr) -> SimpleExpr {
        save_cast(self, value)
    }

    fn compared(&self, value: Expr) -> SimpleExpr {
        compared_value_as(self, value, |value| save_cast(self, value))
    }
}

/// How a disjoint statement writes one column: read through its
/// `select_as`, a value written through its `save_as`, and compared as its
/// value predicates compare it.
// [spec:pgorm:sem:entity.traits.column.enum-cast+17]
#[derive(Clone)]
pub(crate) struct ColumnCasts {
    casts: Arc<dyn Casts>,
    pub(crate) column: Option<TypeName>,
}

impl ColumnCasts {
    pub(super) fn of<C: ColumnTrait>(col: C) -> Self {
        Self {
            column: compared_enum(col.def().get_column_type()),
            casts: Arc::new(Typed(col)),
        }
    }

    pub(super) fn of_type(col_type: &ColumnType) -> Self {
        Self {
            column: compared_enum(col_type),
            casts: Arc::new(col_type.clone()),
        }
    }

    pub(crate) fn select(&self, expr: Expr) -> SimpleExpr {
        self.casts.select(expr)
    }

    pub(crate) fn save(&self, value: Value) -> SimpleExpr {
        self.casts.save(Expr::val(value))
    }

    pub(crate) fn compared(&self, value: Value) -> SimpleExpr {
        self.casts.compared(Expr::val(value))
    }

    /// `column`, a read of the column, as a comparison takes it: cast to the
    /// enum when its domain is over one, as `BoundaryCast` casts a keyset
    /// column.
    pub(crate) fn compared_column(&self, column: Expr) -> Expr {
        match &self.column {
            Some(enum_type) => Expr::expr(column.cast_as_type(enum_type.clone())),
            None => column,
        }
    }

    /// The casts a keyset boundary on the column binds under.
    pub(crate) fn boundary(&self) -> BoundaryCast {
        let casts = Arc::clone(&self.casts);
        BoundaryCast::new(
            Arc::new(move |value| casts.compared(value)),
            self.column.clone(),
        )
    }
}
