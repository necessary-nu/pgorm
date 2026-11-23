//! `COLLATE`: `(operand COLLATE "name")`.

use super::*;

impl QueryBuilder {
    /// Translate a [`SimpleExpr::Collate`] into SQL.
    ///
    /// The clause renders inside its own parentheses, as PostgreSQL's own
    /// deparser writes a `CollateExpr`. Unparenthesised, `COLLATE` is an
    /// `a_expr` production and not an operand everywhere an expression is: a
    /// `BETWEEN` lower bound and a column `DEFAULT` take the narrower `b_expr`,
    /// which has no `COLLATE`, so the first is a syntax error and the second
    /// is worse — `DEFAULT 'a' COLLATE "C"` parses, as the *column's*
    /// collation clause. Wrapped, it is an atom in every position.
    ///
    /// Inside the parentheses the operand is written bare when it already
    /// binds tighter than `COLLATE` — a column, a value, a call, a cast, a
    /// subscript, a self-wrapped `CASE` or subquery — and parenthesised
    /// otherwise, because `COLLATE` binds tighter than every binary operator:
    /// `"a" || "b" COLLATE "C"` collates only `"b"`.
    // [spec:pgorm:req:sql.render.collate+2]
    pub(super) fn prepare_collate(
        &self,
        operand: &SimpleExpr,
        collation: &Collation,
        sql: &mut dyn SqlWriter,
    ) {
        let bare = !matches!(
            operand,
            SimpleExpr::Unary(..)
                | SimpleExpr::Binary(..)
                | SimpleExpr::Raw(_)
                | SimpleExpr::Template(_)
                | SimpleExpr::LikePattern(_)
        );
        write!(sql, "(").unwrap();
        if bare {
            self.prepare_simple_expr(operand, sql);
        } else {
            write!(sql, "(").unwrap();
            self.prepare_simple_expr(operand, sql);
            write!(sql, ")").unwrap();
        }
        write!(sql, " COLLATE ").unwrap();
        collation.prepare(sql.as_writer());
        write!(sql, ")").unwrap();
    }

    /// Write a column's ` COLLATE "name"`, directly after its type, when it
    /// declares one. `CREATE TABLE`, `ADD COLUMN` and the `ALTER COLUMN ...
    /// TYPE` of a modified column all spell it there.
    // [spec:pgorm:req:sql.ddl.column-def+13]
    pub(super) fn prepare_column_collation(&self, column_def: &ColumnDef, sql: &mut dyn SqlWriter) {
        if let Some(collation) = &column_def.collation {
            write!(sql, " COLLATE ").unwrap();
            collation.prepare(sql.as_writer());
        }
    }
}
