//! `GROUP BY` items: a plain expression, a parenthesised set, `()`,
//! `ROLLUP`, `CUBE` and `GROUPING SETS`.

use super::*;
use crate::query::GroupingKind;

impl QueryBuilder {
    /// Translate a [`GroupingElement`] into SQL.
    ///
    /// A set of one expression renders bare, so a `GROUP BY` built only from
    /// the plain group-by methods reads exactly as it did before the list
    /// held elements; any other set renders parenthesised, `()` included. A
    /// `ROLLUP` or `CUBE` with no expressions stands for the one empty set and
    /// renders `()`, because `ROLLUP ()` is not in the grammar.
    // [spec:pgorm:req:sql.render.grouping+2]
    pub(super) fn prepare_grouping_element(
        &self,
        element: &GroupingElement,
        sql: &mut dyn SqlWriter,
    ) {
        match &element.kind {
            GroupingKind::Set(exprs) => match exprs.as_slice() {
                [expr] => self.prepare_simple_expr(expr, sql),
                exprs => self.prepare_tuple(exprs, sql),
            },
            GroupingKind::Rollup(exprs) | GroupingKind::Cube(exprs) if exprs.is_empty() => {
                write!(sql, "()").unwrap();
            }
            GroupingKind::Rollup(exprs) => {
                write!(sql, "ROLLUP ").unwrap();
                self.prepare_tuple(exprs, sql);
            }
            GroupingKind::Cube(exprs) => {
                write!(sql, "CUBE ").unwrap();
                self.prepare_tuple(exprs, sql);
            }
            GroupingKind::Sets(elements) => {
                write!(sql, "GROUPING SETS (").unwrap();
                self.prepare_grouping_list(elements, sql);
                write!(sql, ")").unwrap();
            }
        }
    }

    /// A comma-separated list of grouping elements: the whole `GROUP BY`
    /// list, and the inside of a `GROUPING SETS`.
    // [spec:pgorm:req:sql.render.grouping+2]
    pub(super) fn prepare_grouping_list(
        &self,
        elements: &[GroupingElement],
        sql: &mut dyn SqlWriter,
    ) {
        for (i, element) in elements.iter().enumerate() {
            if i != 0 {
                write!(sql, ", ").unwrap();
            }
            self.prepare_grouping_element(element, sql);
        }
    }
}
