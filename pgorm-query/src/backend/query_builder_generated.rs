//! A generated column's clause, and the `ALTER TABLE` actions that change or
//! drop its expression.

use super::*;

impl QueryBuilder {
    /// Translate the generated column into SQL statement
    ///
    /// The kind is written whichever it is, so the statement says it rather
    /// than leaving it to a release's default: PostgreSQL 19 reads a generated
    /// column without either keyword as `VIRTUAL`, where 17 refused one
    /// (`[spec:pgorm:req:sql.ddl.column-def+14]`).
    // [spec:pgorm:req:sql.ddl.column-def+14]
    pub(crate) fn prepare_generated_column(
        &self,
        gen_: &SimpleExpr,
        kind: GeneratedKind,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "GENERATED ALWAYS AS (").unwrap();
        self.prepare_simple_expr(gen_, sql);
        write!(sql, ") {}", kind.keyword()).unwrap();
    }

    /// `ALTER COLUMN "c" SET EXPRESSION AS (<expr>)`. The `AS` is not
    /// optional: `SET EXPRESSION (<expr>)` is a syntax error (`42601`).
    // [spec:pgorm:req:sql.ddl.alter-table+16]
    pub(crate) fn prepare_set_expression(
        &self,
        column: &Name,
        expr: &SimpleExpr,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "ALTER COLUMN ").unwrap();
        column.prepare(sql.as_writer());
        write!(sql, " SET EXPRESSION AS (").unwrap();
        self.prepare_simple_expr(expr, sql);
        write!(sql, ")").unwrap();
    }

    /// `ALTER COLUMN "c" DROP EXPRESSION[ IF EXISTS]`.
    // [spec:pgorm:req:sql.ddl.alter-table+16]
    pub(crate) fn prepare_drop_expression(
        &self,
        column: &Name,
        if_exists: bool,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "ALTER COLUMN ").unwrap();
        column.prepare(sql.as_writer());
        write!(sql, " DROP EXPRESSION").unwrap();
        if if_exists {
            write!(sql, " IF EXISTS").unwrap();
        }
    }
}
