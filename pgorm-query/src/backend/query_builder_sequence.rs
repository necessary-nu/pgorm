//! `CREATE` / `ALTER` / `DROP SEQUENCE`, and the option list an identity
//! column shares with them.

use super::*;
use crate::sequence::{SequenceClauses, SequenceOwner};

impl QueryBuilder {
    // [spec:pgorm:req:sql.ddl.sequence]
    pub(crate) fn prepare_sequence_create_statement(
        &self,
        create: &SequenceCreateStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "CREATE SEQUENCE ").unwrap();
        if create.if_not_exists {
            write!(sql, "IF NOT EXISTS ").unwrap();
        }
        self.prepare_table_name(&create.name, sql);
        self.prepare_sequence_clauses(&create.clauses, sql);
    }

    // [spec:pgorm:req:sql.ddl.sequence]
    pub(crate) fn prepare_sequence_alter_statement(
        &self,
        alter: &SequenceAlterStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "ALTER SEQUENCE ").unwrap();
        if alter.if_exists {
            write!(sql, "IF EXISTS ").unwrap();
        }
        self.prepare_table_name(&alter.name, sql);
        self.prepare_sequence_clauses(&alter.clauses, sql);
    }

    // [spec:pgorm:req:sql.ddl.sequence]
    pub(crate) fn prepare_sequence_drop_statement(
        &self,
        drop: &SequenceDropStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "DROP SEQUENCE ").unwrap();
        if drop.if_exists {
            write!(sql, "IF EXISTS ").unwrap();
        }
        drop.names().fold(true, |first, name| {
            if !first {
                write!(sql, ", ").unwrap();
            }
            self.prepare_table_name(name, sql);
            false
        });
        if let Some(behavior) = &drop.behavior {
            self.prepare_table_drop_opt(behavior, sql);
        }
    }

    // [spec:pgorm:req:sql.ddl.sequence]
    pub(crate) fn prepare_sequence_rename_statement(
        &self,
        rename: &SequenceRenameStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "ALTER SEQUENCE ").unwrap();
        self.prepare_table_name(&rename.from_name, sql);
        write!(sql, " RENAME TO ").unwrap();
        rename.to_name.prepare(sql.as_writer());
    }

    /// Every clause a create or an alter carries, each with its leading space,
    /// in the one order they render.
    // [spec:pgorm:req:sql.ddl.sequence]
    fn prepare_sequence_clauses(&self, clauses: &SequenceClauses, sql: &mut dyn SqlWriter) {
        if let Some(ty) = clauses.as_type {
            write!(sql, " AS {}", ty.keyword()).unwrap();
        }
        if let Some(options) = &clauses.options {
            write!(sql, " ").unwrap();
            self.prepare_sequence_options(options, sql);
        }
        match clauses.restart {
            Some(Some(value)) => write!(sql, " RESTART WITH {value}").unwrap(),
            Some(None) => write!(sql, " RESTART").unwrap(),
            None => {}
        }
        match &clauses.owned_by {
            Some(SequenceOwner::Column(table, column)) => {
                write!(sql, " OWNED BY ").unwrap();
                self.prepare_table_name(table, sql);
                write!(sql, ".").unwrap();
                column.prepare(sql.as_writer());
            }
            Some(SequenceOwner::Nothing) => write!(sql, " OWNED BY NONE").unwrap(),
            None => {}
        }
    }

    /// The options, space-separated as the grammar lists them, with no
    /// leading or trailing space.
    // [spec:pgorm:req:sql.ddl.sequence]
    pub(super) fn prepare_sequence_options(
        &self,
        options: &SequenceOptions,
        sql: &mut dyn SqlWriter,
    ) {
        options.iter().fold(true, |first, option| {
            if !first {
                write!(sql, " ").unwrap();
            }
            match option {
                SequenceOption::IncrementBy(n) => write!(sql, "INCREMENT BY {n}"),
                SequenceOption::MinValue(n) => write!(sql, "MINVALUE {n}"),
                SequenceOption::NoMinValue => write!(sql, "NO MINVALUE"),
                SequenceOption::MaxValue(n) => write!(sql, "MAXVALUE {n}"),
                SequenceOption::NoMaxValue => write!(sql, "NO MAXVALUE"),
                SequenceOption::StartWith(n) => write!(sql, "START WITH {n}"),
                SequenceOption::Cache(n) => write!(sql, "CACHE {n}"),
                SequenceOption::Cycle => write!(sql, "CYCLE"),
                SequenceOption::NoCycle => write!(sql, "NO CYCLE"),
            }
            .unwrap();
            false
        });
    }

    /// An identity column's ` ( <options> )`, after `AS IDENTITY`, when it has
    /// any. There is no empty form to write: `AS IDENTITY ()` is a syntax
    /// error, and a column with no options has no [`SequenceOptions`].
    // [spec:pgorm:req:sql.ddl.column-def+14]
    pub(super) fn prepare_identity_options(
        &self,
        options: Option<&SequenceOptions>,
        sql: &mut dyn SqlWriter,
    ) {
        if let Some(options) = options {
            write!(sql, " (").unwrap();
            self.prepare_sequence_options(options, sql);
            write!(sql, ")").unwrap();
        }
    }
}
