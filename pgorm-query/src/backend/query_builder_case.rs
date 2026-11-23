//! The two forms of `CASE`: the searched form, whose arms each test a
//! condition, and the simple form, whose arms each hold a value the one
//! operand is compared with. Both render self-parenthesised, `(CASE … END)`,
//! which is what lets `sql.render.precedence` treat either as an atom.

use super::*;

impl QueryBuilder {
    /// Translate [`CaseStatement`] into SQL statement.
    // [spec:pgorm:req:sql.render.case+2]
    pub(super) fn prepare_case_statement(&self, stmts: &CaseStatement, sql: &mut dyn SqlWriter) {
        write!(sql, "(CASE").unwrap();

        let CaseStatement { when, r#else } = stmts;

        for case in when.iter() {
            write!(sql, " WHEN (").unwrap();
            self.prepare_condition_where(&case.condition, sql);
            write!(sql, ") THEN ").unwrap();

            self.prepare_simple_expr(&case.result, sql);
        }
        self.prepare_case_else(r#else.as_ref(), sql);
    }

    /// Translate [`SimpleCaseStatement`] into SQL statement: the operand once,
    /// after `CASE`, then each arm's value bare between `WHEN` and `THEN`.
    /// The keywords delimit every operand, so none needs parentheses.
    // [spec:pgorm:req:sql.render.case+2]
    pub(super) fn prepare_simple_case_statement(
        &self,
        case: &SimpleCaseStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "(CASE ").unwrap();
        self.prepare_simple_expr(&case.operand, sql);

        for arm in case.when.iter() {
            write!(sql, " WHEN ").unwrap();
            self.prepare_simple_expr(&arm.value, sql);
            write!(sql, " THEN ").unwrap();
            self.prepare_simple_expr(&arm.result, sql);
        }
        self.prepare_case_else(case.r#else.as_ref(), sql);
    }

    /// The tail both forms share: the optional `ELSE` result, then `END)`.
    fn prepare_case_else(&self, r#else: Option<&SimpleExpr>, sql: &mut dyn SqlWriter) {
        if let Some(r#else) = r#else {
            write!(sql, " ELSE ").unwrap();
            self.prepare_simple_expr(r#else, sql);
        }

        write!(sql, " END)").unwrap();
    }
}
