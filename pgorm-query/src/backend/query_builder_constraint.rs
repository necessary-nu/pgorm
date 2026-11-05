//! A column's `NOT NULL` clause, and the `ALTER TABLE` actions that add,
//! validate, alter, drop and rename a constraint by name.

use super::*;

impl QueryBuilder {
    /// `[CONSTRAINT "name" ]NOT NULL[ NO INHERIT]`: a column's not-null
    /// constraint, the clause `CREATE TABLE` and `ADD COLUMN` write it with.
    // [spec:pgorm:req:sql.ddl.column-def+13]
    pub(crate) fn prepare_not_null(
        &self,
        name: Option<&Name>,
        no_inherit: bool,
        sql: &mut dyn SqlWriter,
    ) {
        self.prepare_constraint_name(name, sql);
        write!(sql, "NOT NULL").unwrap();
        if no_inherit {
            write!(sql, " NO INHERIT").unwrap();
        }
    }

    /// `ADD [CONSTRAINT "name" ]NOT NULL "c"[ NO INHERIT][ NOT VALID]`: the
    /// table-level spelling, and the only one with a place for `NOT VALID`.
    /// A modified column's named or `NO INHERIT` constraint is written this
    /// way too, as `SET NOT NULL` can carry neither.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub(crate) fn prepare_add_not_null(
        &self,
        constraint: &NotNullConstraint,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "ADD ").unwrap();
        self.prepare_constraint_name(constraint.name.as_ref(), sql);
        write!(sql, "NOT NULL ").unwrap();
        constraint.column.prepare(sql.as_writer());
        if constraint.no_inherit {
            write!(sql, " NO INHERIT").unwrap();
        }
        if constraint.not_valid {
            write!(sql, " NOT VALID").unwrap();
        }
    }

    /// ` NOT VALID` after a constraint an `ALTER TABLE ... ADD` adds, when it
    /// leaves the rows already there unchecked.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub(crate) fn prepare_not_valid(&self, not_valid: bool, sql: &mut dyn SqlWriter) {
        if not_valid {
            write!(sql, " NOT VALID").unwrap();
        }
    }

    /// `VALIDATE CONSTRAINT "name"`.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub(crate) fn prepare_validate_constraint(&self, name: &Name, sql: &mut dyn SqlWriter) {
        write!(sql, "VALIDATE CONSTRAINT ").unwrap();
        name.prepare(sql.as_writer());
    }

    /// `ALTER CONSTRAINT "name" <change>`.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub(crate) fn prepare_alter_constraint(
        &self,
        name: &Name,
        change: ConstraintChange,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "ALTER CONSTRAINT ").unwrap();
        name.prepare(sql.as_writer());
        write!(sql, " {}", change.clause()).unwrap();
    }

    /// `DROP CONSTRAINT [IF EXISTS ]"name"[ RESTRICT | CASCADE]`.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub(crate) fn prepare_drop_constraint(&self, drop: &ConstraintDrop, sql: &mut dyn SqlWriter) {
        write!(sql, "DROP CONSTRAINT ").unwrap();
        if drop.if_exists {
            write!(sql, "IF EXISTS ").unwrap();
        }
        drop.name.prepare(sql.as_writer());
        if let Some(behavior) = drop.behavior {
            write!(sql, " {}", behavior.keyword()).unwrap();
        }
    }

    /// `ALTER TABLE <table> RENAME CONSTRAINT "from" TO "to"`.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub(crate) fn prepare_constraint_rename_statement(
        &self,
        rename: &ConstraintRenameStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "ALTER TABLE ").unwrap();
        self.prepare_table_name(&rename.table, sql);
        write!(sql, " RENAME CONSTRAINT ").unwrap();
        rename.from_name.prepare(sql.as_writer());
        write!(sql, " TO ").unwrap();
        rename.to_name.prepare(sql.as_writer());
    }

    /// `CONSTRAINT "name" `, when the constraint has a name; nothing when the
    /// server is to derive one.
    pub(super) fn prepare_constraint_name(&self, name: Option<&Name>, sql: &mut dyn SqlWriter) {
        if let Some(name) = name {
            write!(sql, "CONSTRAINT ").unwrap();
            name.prepare(sql.as_writer());
            write!(sql, " ").unwrap();
        }
    }
}
