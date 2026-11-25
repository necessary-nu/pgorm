//! `CREATE [OR REPLACE] [RECURSIVE] VIEW`, the materialized view's three
//! statements, `CREATE TABLE AS` and the drops: a named relation over a
//! `SELECT`.

use super::*;
use crate::view::{RefreshMode, RelationDrop, ViewDefinition};

impl QueryBuilder {
    // [spec:pgorm:req:sql.ddl.view+2]
    pub(crate) fn prepare_view_create_statement(
        &self,
        create: &ViewCreateStatement,
        sql: &mut dyn SqlWriter,
    ) {
        self.prepare_view_definition(&create.definition, "", sql);
        if let Some(option) = create.check_option {
            write!(sql, " WITH {} CHECK OPTION", option.keyword()).unwrap();
        }
    }

    // [spec:pgorm:req:sql.ddl.view+2]
    pub(crate) fn prepare_recursive_view_create_statement(
        &self,
        create: &RecursiveViewCreateStatement,
        sql: &mut dyn SqlWriter,
    ) {
        self.prepare_view_definition(&create.definition, "RECURSIVE ", sql);
    }

    /// `CREATE [OR REPLACE ]<recursive>VIEW <name>[ (<columns>)][ WITH
    /// (<options>)] AS <query>`: everything a view statement writes before
    /// its check option.
    // [spec:pgorm:req:sql.ddl.view+2]
    fn prepare_view_definition(
        &self,
        definition: &ViewDefinition,
        recursive: &str,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "CREATE ").unwrap();
        if definition.or_replace {
            write!(sql, "OR REPLACE ").unwrap();
        }
        write!(sql, "{recursive}VIEW ").unwrap();
        self.prepare_table_name(&definition.name, sql);
        self.prepare_relation_columns(&definition.columns, sql);
        let options = [
            (definition.security_barrier, "security_barrier"),
            (definition.security_invoker, "security_invoker"),
        ];
        options
            .iter()
            .filter(|(set, _)| *set)
            .fold(true, |first, (_, option)| {
                write!(sql, "{}{option}", if first { " WITH (" } else { ", " }).unwrap();
                false
            });
        if options.iter().any(|(set, _)| *set) {
            write!(sql, ")").unwrap();
        }
        self.prepare_relation_body(&definition.body, sql);
    }

    // [spec:pgorm:req:sql.ddl.view+2]
    pub(crate) fn prepare_view_drop_statement(
        &self,
        drop: &ViewDropStatement,
        sql: &mut dyn SqlWriter,
    ) {
        self.prepare_relation_drop("VIEW", &drop.relations, sql);
    }

    // [spec:pgorm:req:sql.ddl.materialized-view+2]
    pub(crate) fn prepare_materialized_view_create_statement(
        &self,
        create: &MaterializedViewCreateStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "CREATE MATERIALIZED VIEW ").unwrap();
        if create.if_not_exists {
            write!(sql, "IF NOT EXISTS ").unwrap();
        }
        self.prepare_table_name(&create.name, sql);
        self.prepare_relation_columns(&create.columns, sql);
        self.prepare_relation_body(&create.body, sql);
        if create.no_data {
            write!(sql, " WITH NO DATA").unwrap();
        }
    }

    // [spec:pgorm:req:sql.ddl.materialized-view+2]
    pub(crate) fn prepare_materialized_view_refresh_statement(
        &self,
        refresh: &MaterializedViewRefreshStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "REFRESH MATERIALIZED VIEW ").unwrap();
        if refresh.mode == Some(RefreshMode::Concurrently) {
            write!(sql, "CONCURRENTLY ").unwrap();
        }
        self.prepare_table_name(&refresh.name, sql);
        if refresh.mode == Some(RefreshMode::NoData) {
            write!(sql, " WITH NO DATA").unwrap();
        }
    }

    // [spec:pgorm:req:sql.ddl.materialized-view+2]
    pub(crate) fn prepare_materialized_view_drop_statement(
        &self,
        drop: &MaterializedViewDropStatement,
        sql: &mut dyn SqlWriter,
    ) {
        self.prepare_relation_drop("MATERIALIZED VIEW", &drop.relations, sql);
    }

    // [spec:pgorm:req:sql.ddl.create-table-as+1]
    pub(crate) fn prepare_table_create_as_statement(
        &self,
        create: &TableCreateAsStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "CREATE TABLE ").unwrap();
        if create.if_not_exists {
            write!(sql, "IF NOT EXISTS ").unwrap();
        }
        self.prepare_table_name(&create.name, sql);
        self.prepare_relation_columns(&create.columns, sql);
        self.prepare_relation_body(&create.body, sql);
        if create.no_data {
            write!(sql, " WITH NO DATA").unwrap();
        }
    }

    /// ` ("a", "b")` after a relation's name, or nothing for no names: the
    /// grammar has no empty column list.
    fn prepare_relation_columns(&self, columns: &[Name], sql: &mut dyn SqlWriter) {
        columns.iter().fold(true, |first, column| {
            write!(sql, "{}", if first { " (" } else { ", " }).unwrap();
            column.prepare(sql.as_writer());
            false
        });
        if !columns.is_empty() {
            write!(sql, ")").unwrap();
        }
    }

    /// ` AS <query>`, the query written through the sink it is handed. Every
    /// public route here hands it a `String`, which writes each value as an
    /// escaped literal: a definition is stored as text and binds nothing.
    fn prepare_relation_body(&self, body: &SelectStatement, sql: &mut dyn SqlWriter) {
        write!(sql, " AS ").unwrap();
        self.prepare_select_statement(body, sql);
    }

    /// `DROP <kind> [IF EXISTS ]<name>[, <name>...][ <behavior>]`.
    fn prepare_relation_drop(&self, kind: &str, drop: &RelationDrop, sql: &mut dyn SqlWriter) {
        write!(sql, "DROP {kind} ").unwrap();
        if drop.if_exists {
            write!(sql, "IF EXISTS ").unwrap();
        }
        let names = std::iter::once(&drop.first).chain(&drop.rest);
        names.fold(true, |first, name| {
            if !first {
                write!(sql, ", ").unwrap();
            }
            self.prepare_table_name(name, sql);
            false
        });
        if let Some(behavior) = drop.behavior {
            write!(sql, " {}", behavior.keyword()).unwrap();
        }
    }
}
