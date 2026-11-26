//! A composite's attribute list in `CREATE TYPE ... AS (...)`, and the
//! `ALTER TYPE` changes to it.

use super::*;
use crate::extension::{
    AttributeChange, AttributeRenameStatement, CompositeAlterStatement, CompositeAttribute,
};

impl QueryBuilder {
    /// A composite's `(<attribute>, ...)`. The parentheses are written for an
    /// empty list too: `AS ()` is the empty composite, and `AS` alone is no
    /// statement.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    pub(super) fn prepare_composite_attributes(
        &self,
        attributes: &[CompositeAttribute],
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "(").unwrap();
        attributes.iter().fold(true, |first, attribute| {
            if !first {
                write!(sql, ", ").unwrap();
            }
            self.prepare_composite_attribute(attribute, sql);
            false
        });
        write!(sql, ")").unwrap();
    }

    /// One attribute: its quoted name, the type as a column writes it, and
    /// its collation when it has one.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    fn prepare_composite_attribute(&self, attribute: &CompositeAttribute, sql: &mut dyn SqlWriter) {
        attribute.name.prepare(sql.as_writer());
        write!(sql, " ").unwrap();
        self.prepare_attribute_type(attribute, sql);
    }

    /// An attribute's type and the collation it takes.
    fn prepare_attribute_type(&self, attribute: &CompositeAttribute, sql: &mut dyn SqlWriter) {
        self.prepare_column_type(&attribute.column_type, sql);
        if let Some(collation) = &attribute.collation {
            write!(sql, " COLLATE ").unwrap();
            collation.prepare(sql.as_writer());
        }
    }

    /// `ALTER TYPE <type> <change>[ <behavior>], ...`, the behavior written
    /// after each change.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    pub(crate) fn prepare_composite_alter_statement(
        &self,
        alter: &CompositeAlterStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "ALTER TYPE ").unwrap();
        self.prepare_type_ref(&alter.name, sql);
        alter.changes().fold(true, |first, change| {
            write!(sql, "{}", if first { " " } else { ", " }).unwrap();
            self.prepare_attribute_change(change, sql);
            if let Some(behavior) = alter.behavior {
                write!(sql, " {}", behavior.keyword()).unwrap();
            }
            false
        });
    }

    fn prepare_attribute_change(&self, change: &AttributeChange, sql: &mut dyn SqlWriter) {
        match change {
            AttributeChange::Add(attribute) => {
                write!(sql, "ADD ATTRIBUTE ").unwrap();
                self.prepare_composite_attribute(attribute, sql);
            }
            AttributeChange::Drop { name, if_exists } => {
                write!(sql, "DROP ATTRIBUTE ").unwrap();
                if *if_exists {
                    write!(sql, "IF EXISTS ").unwrap();
                }
                name.prepare(sql.as_writer());
            }
            AttributeChange::Retype(attribute) => {
                write!(sql, "ALTER ATTRIBUTE ").unwrap();
                attribute.name.prepare(sql.as_writer());
                write!(sql, " TYPE ").unwrap();
                self.prepare_attribute_type(attribute, sql);
            }
        }
    }

    /// `ALTER TYPE <type> RENAME ATTRIBUTE "from" TO "to"[ <behavior>]`.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    pub(crate) fn prepare_attribute_rename_statement(
        &self,
        rename: &AttributeRenameStatement,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "ALTER TYPE ").unwrap();
        self.prepare_type_ref(&rename.name, sql);
        write!(sql, " RENAME ATTRIBUTE ").unwrap();
        rename.from.prepare(sql.as_writer());
        write!(sql, " TO ").unwrap();
        rename.to.prepare(sql.as_writer());
        if let Some(behavior) = rename.behavior {
            write!(sql, " {}", behavior.keyword()).unwrap();
        }
    }
}
