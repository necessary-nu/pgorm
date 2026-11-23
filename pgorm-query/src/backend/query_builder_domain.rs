//! `CREATE DOMAIN`, `ALTER DOMAIN` and `DROP DOMAIN`, and the `b_expr` operand
//! rule a domain's default shares with a column definition's.

use super::*;
use crate::domain::DomainAlterAction;

impl QueryBuilder {
    // [spec:pgorm:req:sql.ddl.domain+4]
    pub(crate) fn prepare_domain_create_statement(
        &self,
        create: &DomainCreateStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "CREATE DOMAIN ").unwrap();
        self.prepare_type_ref(&create.name, sql);
        write!(sql, " AS ").unwrap();
        self.prepare_column_type(&create.base, sql);
        if let Some(collation) = &create.collation {
            write!(sql, " COLLATE ").unwrap();
            collation.prepare(sql.as_writer());
        }
        if let Some(default) = &create.default {
            write!(sql, " DEFAULT ").unwrap();
            self.prepare_default_operand(default, sql);
        }
        match create.not_null {
            Some(true) => write!(sql, " NOT NULL").unwrap(),
            Some(false) => write!(sql, " NULL").unwrap(),
            None => {}
        }
        for check in &create.checks {
            write!(sql, " ").unwrap();
            self.prepare_domain_check(check, sql);
        }
    }

    // [spec:pgorm:req:sql.ddl.domain+4]
    pub(crate) fn prepare_domain_alter_statement(
        &self,
        alter: &DomainAlterStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "ALTER DOMAIN ").unwrap();
        self.prepare_type_ref(&alter.name, sql);
        write!(sql, " ").unwrap();
        match &alter.action {
            DomainAlterAction::SetDefault(expr) => {
                write!(sql, "SET DEFAULT ").unwrap();
                self.prepare_simple_expr(expr, sql);
            }
            DomainAlterAction::DropDefault => write!(sql, "DROP DEFAULT").unwrap(),
            DomainAlterAction::SetNotNull => write!(sql, "SET NOT NULL").unwrap(),
            DomainAlterAction::DropNotNull => write!(sql, "DROP NOT NULL").unwrap(),
            DomainAlterAction::AddCheck(check, not_valid) => {
                write!(sql, "ADD ").unwrap();
                self.prepare_domain_check(check, sql);
                self.prepare_not_valid(*not_valid, sql);
            }
            DomainAlterAction::DropConstraint(drop) => self.prepare_drop_constraint(drop, sql),
            DomainAlterAction::RenameConstraint(from, to) => {
                write!(sql, "RENAME CONSTRAINT ").unwrap();
                from.prepare(sql.as_writer());
                write!(sql, " TO ").unwrap();
                to.prepare(sql.as_writer());
            }
            DomainAlterAction::ValidateConstraint(name) => {
                self.prepare_validate_constraint(name, sql)
            }
            DomainAlterAction::Rename(name) => {
                write!(sql, "RENAME TO ").unwrap();
                name.prepare(sql.as_writer());
            }
        }
    }

    // [spec:pgorm:req:sql.ddl.domain+4]
    pub(crate) fn prepare_domain_drop_statement(
        &self,
        drop: &DomainDropStatement,
        sql: &mut dyn SqlWriter,
    ) {
        self.prepare_type_names_drop("DOMAIN", drop.if_exists, drop.names(), sql);
        if let Some(behavior) = drop.behavior {
            write!(sql, " {}", behavior.keyword()).unwrap();
        }
    }

    /// `DROP <kind> [IF EXISTS ]<type>[, <type>...]`: what a drop of types
    /// named as types are writes before its behavior, a domain's and an enum
    /// or composite type's alike.
    // [spec:pgorm:req:sql.ddl.domain+4]
    // [spec:pgorm:req:sql.ddl.type-alter-drop+7]
    pub(super) fn prepare_type_names_drop<'a>(
        &self,
        kind: &str,
        if_exists: bool,
        names: impl IntoIterator<Item = &'a TypeRef>,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "DROP {kind} ").unwrap();
        if if_exists {
            write!(sql, "IF EXISTS ").unwrap();
        }
        for (index, name) in names.into_iter().enumerate() {
            if index > 0 {
                write!(sql, ", ").unwrap();
            }
            self.prepare_type_ref(name, sql);
        }
    }

    /// `[CONSTRAINT "name" ]CHECK (<expr>)`.
    fn prepare_domain_check(&self, check: &DomainCheck, sql: &mut dyn SqlWriter) {
        self.prepare_constraint_name(check.name.as_ref(), sql);
        write!(sql, "CHECK (").unwrap();
        self.prepare_simple_expr(&check.expr, sql);
        write!(sql, ")").unwrap();
    }

    /// A default where the grammar reads a `b_expr` — a column definition's
    /// `DEFAULT`, in `CREATE TABLE` and `ADD COLUMN`, and `CREATE DOMAIN`'s:
    /// the expression narrower than every other operand, with no logical
    /// operator, `IS`, `BETWEEN`, `IN` or `LIKE` (`42601` at the operator).
    /// An operator expression is parenthesised so that it is an operand there
    /// whatever its operator; anything else is one already. `ALTER COLUMN ...
    /// SET DEFAULT` and `ALTER DOMAIN ... SET DEFAULT` read the full `a_expr`
    /// and need none of this.
    // [spec:pgorm:req:sql.ddl.column-def+14]
    // [spec:pgorm:req:sql.ddl.domain+4]
    pub(crate) fn prepare_default_operand(&self, default: &SimpleExpr, sql: &mut dyn SqlWriter) {
        let operand = !matches!(
            default,
            SimpleExpr::Unary(..)
                | SimpleExpr::Binary(..)
                | SimpleExpr::Raw(_)
                | SimpleExpr::Template(_)
                | SimpleExpr::LikePattern(_)
        );
        if operand {
            self.prepare_simple_expr(default, sql);
        } else {
            write!(sql, "(").unwrap();
            self.prepare_simple_expr(default, sql);
            write!(sql, ")").unwrap();
        }
    }
}
