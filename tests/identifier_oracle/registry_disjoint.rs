//! The registry, continued: the disjoint schema and its changes, and the
//! one-of `CHECK`. See `registry.rs` for how to add a site.
//!
//! A disjoint entity's statements are written from a [`DisjointShape`],
//! which a binding describes at run time with every table, column, type and
//! value a name it was handed (`DisjointShape::new`), so each name is held
//! at every statement it reaches: one site per statement and name, the
//! other names fixed ([`disjoint_named`]). The names pgorm composes from
//! them — the `uq-`, `fk-` and `ck-` constraints, the generated key columns
//! `{value}_{column}`, the `{type}_next` type a change recreates the kind
//! type as — are positions too, held to the bounded derivation.
//!
//! [`DisjointShape`]: pgorm::DisjointShape

use pgorm::Schema;

use super::{
    fixtures::{
        disjoint_named::{self, Slot},
        one_of_named,
    },
    oracle::{
        Policy::{DerivedLabel, Literal, Quoted, TableScoped, TypePart},
        Rendered, Site,
    },
};

pub fn sites() -> Vec<Site> {
    let mut sites = disjoint_sites();
    sites.extend(one_of_sites());
    sites
}

fn disjoint_sites() -> Vec<Site> {
    vec![
        Site {
            id: "pgorm/disjoint.create.type.kind-type",
            api: "DisjointShape::create — the kind type's CREATE TYPE, its name",
            kinds: &["CreateEnumStmt.type_name[0]"],
            policy: Quoted,
            render: |n| disjoint_named::create(Slot::KindType, n, 0),
        },
        Site {
            id: "pgorm/disjoint.create.type.type-schema",
            api: "DisjointShape::create — the kind type's CREATE TYPE, its schema",
            kinds: &["CreateEnumStmt.type_name[0]"],
            policy: Quoted,
            render: |n| disjoint_named::create(Slot::TypeSchema, n, 0),
        },
        Site {
            id: "pgorm/disjoint.create.type.value",
            api: "DisjointShape::create — the kind type's CREATE TYPE, a kind's value",
            kinds: &["CreateEnumStmt.vals[0]"],
            policy: Literal,
            render: |n| disjoint_named::create(Slot::Value, n, 0),
        },
        Site {
            id: "pgorm/disjoint.create.base.table",
            api: "DisjointShape::create — the base's CREATE TABLE, its name",
            kinds: &["CreateStmt.relation.relname", "Constraint.conname"],
            policy: Quoted,
            render: |n| disjoint_named::create(Slot::Base, n, 1),
        },
        Site {
            id: "pgorm/disjoint.create.base.key",
            api: "DisjointShape::create — the base's CREATE TABLE, its key column",
            kinds: &[
                "ColumnDef.colname",
                "Constraint.keys[0]",
                "Constraint.conname",
                "Constraint.keys[0]",
            ],
            policy: Quoted,
            render: |n| disjoint_named::create(Slot::Key, n, 1),
        },
        Site {
            id: "pgorm/disjoint.create.base.kind-column",
            api: "DisjointShape::create — the base's CREATE TABLE, its kind column",
            kinds: &[
                "ColumnDef.colname",
                "Constraint.conname",
                "Constraint.keys[1]",
            ],
            policy: Quoted,
            render: |n| disjoint_named::create(Slot::KindColumn, n, 1),
        },
        Site {
            id: "pgorm/disjoint.create.base.kind-type",
            api: "DisjointShape::create — the base's CREATE TABLE, the kind type",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| disjoint_named::create(Slot::KindType, n, 1),
        },
        Site {
            id: "pgorm/disjoint.create.base.type-schema",
            api: "DisjointShape::create — the base's CREATE TABLE, the kind type's schema",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| disjoint_named::create(Slot::TypeSchema, n, 1),
        },
        Site {
            id: "pgorm/disjoint.create.kind-table.table",
            api: "DisjointShape::create — a kind's CREATE TABLE, its name",
            kinds: &["CreateStmt.relation.relname", "Constraint.conname"],
            policy: Quoted,
            render: |n| disjoint_named::create(Slot::KindTable, n, 2),
        },
        Site {
            id: "pgorm/disjoint.create.kind-table.base",
            api: "DisjointShape::create — a kind's CREATE TABLE, the base its key references",
            kinds: &["Constraint.pktable.relname"],
            policy: Quoted,
            render: |n| disjoint_named::create(Slot::Base, n, 2),
        },
        Site {
            id: "pgorm/disjoint.create.kind-table.key",
            api: "DisjointShape::create — a kind's CREATE TABLE, its key column",
            kinds: &[
                "ColumnDef.colname",
                "Constraint.keys[0]",
                "Constraint.conname",
                "Constraint.fk_attrs[0]",
                "Constraint.pk_attrs[0]",
            ],
            policy: Quoted,
            render: |n| disjoint_named::create(Slot::Key, n, 2),
        },
        Site {
            id: "pgorm/disjoint.create.kind-table.kind-column",
            api: "DisjointShape::create — a kind's CREATE TABLE, its kind column",
            kinds: &[
                "ColumnDef.colname",
                "Constraint.conname",
                "Constraint.fk_attrs[1]",
                "Constraint.pk_attrs[1]",
            ],
            policy: Quoted,
            render: |n| disjoint_named::create(Slot::KindColumn, n, 2),
        },
        Site {
            id: "pgorm/disjoint.create.kind-table.kind-type",
            api: "DisjointShape::create — a kind's CREATE TABLE, the kind type",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| disjoint_named::create(Slot::KindType, n, 2),
        },
        Site {
            id: "pgorm/disjoint.create.kind-table.value",
            api: "DisjointShape::create — a kind's CREATE TABLE, its value",
            kinds: &["AConst.val"],
            policy: Literal,
            render: |n| disjoint_named::create(Slot::Value, n, 2),
        },
        Site {
            id: "pgorm/disjoint.create.keys.base",
            api: "DisjointShape::create — the base's generated key columns, the base",
            kinds: &["Constraint.conname", "AlterTableStmt.relation.relname"],
            policy: Quoted,
            render: |n| disjoint_named::create(Slot::Base, n, 3),
        },
        Site {
            id: "pgorm/disjoint.create.keys.kind-table",
            api: "DisjointShape::create — the base's generated key columns, a kind's table",
            kinds: &["Constraint.pktable.relname"],
            policy: Quoted,
            render: |n| disjoint_named::create(Slot::KindTable, n, 3),
        },
        Site {
            id: "pgorm/disjoint.create.keys.key",
            api: "DisjointShape::create — the base's generated key columns, the key column",
            kinds: &[
                "ColumnDef.colname",
                "ColumnRef.fields[0]",
                "Constraint.conname",
                "Constraint.fk_attrs[0]",
                "Constraint.pk_attrs[0]",
            ],
            policy: Quoted,
            render: |n| disjoint_named::create(Slot::Key, n, 3),
        },
        Site {
            id: "pgorm/disjoint.create.keys.kind-column",
            api: "DisjointShape::create — the base's generated key columns, the kind column",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| disjoint_named::create(Slot::KindColumn, n, 3),
        },
        Site {
            id: "pgorm/disjoint.create.keys.value",
            api: "DisjointShape::create — the base's generated key columns, a kind's value",
            kinds: &[
                "ColumnDef.colname",
                "AConst.val",
                "Constraint.conname",
                "Constraint.fk_attrs[0]",
            ],
            policy: DerivedLabel,
            render: |n| disjoint_named::create(Slot::Value, n, 3),
        },
        Site {
            id: "pgorm/disjoint.add-kind.value.kind-type",
            api: "DisjointShape::add_kind — ALTER TYPE ADD VALUE, the kind type",
            kinds: &["AlterEnumStmt.type_name[0]"],
            policy: Quoted,
            render: |n| disjoint_named::add_kind(Slot::KindType, n, (0, 0)),
        },
        Site {
            id: "pgorm/disjoint.add-kind.value.type-schema",
            api: "DisjointShape::add_kind — ALTER TYPE ADD VALUE, the kind type's schema",
            kinds: &["AlterEnumStmt.type_name[0]"],
            policy: Quoted,
            render: |n| disjoint_named::add_kind(Slot::TypeSchema, n, (0, 0)),
        },
        Site {
            id: "pgorm/disjoint.add-kind.value.value",
            api: "DisjointShape::add_kind — ALTER TYPE ADD VALUE, the value",
            kinds: &["AlterEnumStmt.new_val"],
            policy: Literal,
            render: |n| disjoint_named::add_kind(Slot::Value, n, (0, 0)),
        },
        Site {
            id: "pgorm/disjoint.add-kind.keys.value",
            api: "DisjointShape::add_kind — the base's generated key columns, the value",
            kinds: &[
                "ColumnDef.colname",
                "AConst.val",
                "Constraint.conname",
                "Constraint.fk_attrs[0]",
            ],
            policy: DerivedLabel,
            render: |n| disjoint_named::add_kind(Slot::Value, n, (1, 1)),
        },
        Site {
            id: "pgorm/disjoint.add-kind-recreating.retype.value",
            api: "DisjointShape::add_kind_recreating — the base retyped, the added value",
            kinds: &["ColumnDef.colname", "AConst.val"],
            policy: DerivedLabel,
            render: |n| disjoint_named::add_kind_recreating(Slot::Value, n, 1),
        },
        Site {
            id: "pgorm/disjoint.add-kind-recreating.retype.kind-type",
            api: "DisjointShape::add_kind_recreating — the base retyped, the `{type}_next` type",
            kinds: &[
                "TypeCast.type_name.names[0]",
                "ColumnDef.type_name.names[0]",
            ],
            policy: TypePart,
            render: |n| disjoint_named::add_kind_recreating(Slot::KindType, n, 1),
        },
        Site {
            id: "pgorm/disjoint.add-kind-recreating.keys.value",
            api: "DisjointShape::add_kind_recreating — the base's foreign keys, the added value",
            kinds: &["Constraint.conname", "Constraint.fk_attrs[0]"],
            policy: Quoted,
            render: |n| disjoint_named::add_kind_recreating(Slot::Value, n, 6),
        },
        Site {
            id: "pgorm/disjoint.add-kind-recreating.keys.kind-table",
            api: "DisjointShape::add_kind_recreating — the base's foreign keys, the added table",
            kinds: &["Constraint.pktable.relname"],
            policy: Quoted,
            render: |n| disjoint_named::add_kind_recreating(Slot::KindTable, n, 6),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.check.base",
            api: "DisjointShape::remove_kind — the CHECK refusing the value, the base",
            kinds: &["Constraint.conname", "AlterTableStmt.relation.relname"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::Base, n, 0),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.check.kind-column",
            api: "DisjointShape::remove_kind — the CHECK refusing the value, the kind column",
            kinds: &["Constraint.conname", "ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::KindColumn, n, 0),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.check.value",
            api: "DisjointShape::remove_kind — the CHECK refusing the value, the value",
            kinds: &["Constraint.conname", "AConst.val"],
            policy: DerivedLabel,
            render: |n| disjoint_named::remove_kind(Slot::Removed, n, 0),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.check-drop.value",
            api: "DisjointShape::remove_kind — the CHECK dropped, the value",
            kinds: &["AlterTableCmd.name"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::Removed, n, 1),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.next.kind-type",
            api: "DisjointShape::remove_kind — CREATE TYPE `{type}_next`",
            kinds: &["CreateEnumStmt.type_name[0]"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::KindType, n, 2),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.next.type-schema",
            api: "DisjointShape::remove_kind — CREATE TYPE `{type}_next`, its schema",
            kinds: &["CreateEnumStmt.type_name[0]"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::TypeSchema, n, 2),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.next.value",
            api: "DisjointShape::remove_kind — CREATE TYPE `{type}_next`, a kept value",
            kinds: &["CreateEnumStmt.vals[0]"],
            policy: Literal,
            render: |n| disjoint_named::remove_kind(Slot::Value, n, 2),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.kind-table-drop.table",
            api: "DisjointShape::remove_kind — a kind table's kind column dropped, its table",
            kinds: &["AlterTableCmd.name", "AlterTableStmt.relation.relname"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::KindTable, n, 3),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.kind-table-drop.key",
            api: "DisjointShape::remove_kind — a kind table's kind column dropped, its key",
            kinds: &["AlterTableCmd.name"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::Key, n, 3),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.kind-table-drop.kind-column",
            api: "DisjointShape::remove_kind — a kind table's kind column dropped, the kind column",
            kinds: &["AlterTableCmd.name", "AlterTableCmd.name"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::KindColumn, n, 3),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.retype.base",
            api: "DisjointShape::remove_kind — the base retyped, the base",
            kinds: &[
                "AlterTableCmd.name",
                "AlterTableCmd.name",
                "AlterTableStmt.relation.relname",
            ],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::Base, n, 4),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.retype.key",
            api: "DisjointShape::remove_kind — the base retyped, the key column",
            kinds: &[
                "AlterTableCmd.name",
                "AlterTableCmd.name",
                "AlterTableCmd.name",
                "ColumnDef.colname",
                "ColumnRef.fields[0]",
            ],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::Key, n, 4),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.retype.kind-column",
            api: "DisjointShape::remove_kind — the base retyped, the kind column",
            kinds: &[
                "AlterTableCmd.name",
                "AlterTableCmd.name",
                "ColumnRef.fields[0]",
                "AlterTableCmd.name",
                "ColumnRef.fields[0]",
            ],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::KindColumn, n, 4),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.retype.kind-type",
            api: "DisjointShape::remove_kind — the base retyped, the `{type}_next` type",
            kinds: &[
                "TypeCast.type_name.names[0]",
                "ColumnDef.type_name.names[0]",
            ],
            policy: TypePart,
            render: |n| disjoint_named::remove_kind(Slot::KindType, n, 4),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.retype.type-schema",
            api: "DisjointShape::remove_kind — the base retyped, the kind type's schema",
            kinds: &[
                "TypeCast.type_name.names[0]",
                "ColumnDef.type_name.names[0]",
            ],
            policy: TypePart,
            render: |n| disjoint_named::remove_kind(Slot::TypeSchema, n, 4),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.retype.value",
            api: "DisjointShape::remove_kind — the base retyped, a kept value",
            kinds: &[
                "AlterTableCmd.name",
                "AlterTableCmd.name",
                "ColumnDef.colname",
                "AConst.val",
            ],
            policy: DerivedLabel,
            render: |n| disjoint_named::remove_kind(Slot::Value, n, 4),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.drop-type.kind-type",
            api: "DisjointShape::remove_kind — DROP TYPE, the kind type",
            kinds: &["TypeName.names[0]"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::KindType, n, 5),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.drop-type.type-schema",
            api: "DisjointShape::remove_kind — DROP TYPE, the kind type's schema",
            kinds: &["TypeName.names[0]"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::TypeSchema, n, 5),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.rename.kind-type",
            api: "DisjointShape::remove_kind — `{type}_next` renamed to the kind type",
            kinds: &["RenameStmt.newname", "RenameStmt.object.List.items[0]"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::KindType, n, 6),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.rename.type-schema",
            api: "DisjointShape::remove_kind — `{type}_next` renamed, the kind type's schema",
            kinds: &["RenameStmt.object.List.items[0]"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::TypeSchema, n, 6),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.restore.base",
            api: "DisjointShape::remove_kind — the base's UNIQUE key restored, the base",
            kinds: &["Constraint.conname", "AlterTableStmt.relation.relname"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::Base, n, 7),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.restore.key",
            api: "DisjointShape::remove_kind — the base's UNIQUE key restored, the key column",
            kinds: &["Constraint.conname", "Constraint.keys[0]"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::Key, n, 7),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.restore.kind-column",
            api: "DisjointShape::remove_kind — the base's UNIQUE key restored, the kind column",
            kinds: &["Constraint.conname", "Constraint.keys[1]"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::KindColumn, n, 7),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.kind-table-add.table",
            api: "DisjointShape::remove_kind — a kind table's kind column written again, its table",
            kinds: &["Constraint.conname", "AlterTableStmt.relation.relname"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::KindTable, n, 8),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.kind-table-add.base",
            api: "DisjointShape::remove_kind — a kind table's kind column written again, the base",
            kinds: &["Constraint.pktable.relname"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::Base, n, 8),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.kind-table-add.key",
            api: "DisjointShape::remove_kind — a kind table's kind column written again, its key",
            kinds: &[
                "Constraint.conname",
                "Constraint.fk_attrs[0]",
                "Constraint.pk_attrs[0]",
            ],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::Key, n, 8),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.kind-table-add.kind-column",
            api: "DisjointShape::remove_kind — a kind table's kind column written again, the kind column",
            kinds: &[
                "ColumnDef.colname",
                "Constraint.conname",
                "Constraint.fk_attrs[1]",
                "Constraint.pk_attrs[1]",
            ],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::KindColumn, n, 8),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.kind-table-add.kind-type",
            api: "DisjointShape::remove_kind — a kind table's kind column written again, the kind type",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| disjoint_named::remove_kind(Slot::KindType, n, 8),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.kind-table-add.value",
            api: "DisjointShape::remove_kind — a kind table's kind column written again, its value",
            kinds: &["AConst.val"],
            policy: Literal,
            render: |n| disjoint_named::remove_kind(Slot::Value, n, 8),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.keys.base",
            api: "DisjointShape::remove_kind — the base's foreign keys, the base",
            kinds: &["Constraint.conname", "AlterTableStmt.relation.relname"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::Base, n, 9),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.keys.kind-table",
            api: "DisjointShape::remove_kind — the base's foreign keys, a kind's table",
            kinds: &["Constraint.pktable.relname"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::KindTable, n, 9),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.keys.key",
            api: "DisjointShape::remove_kind — the base's foreign keys, the key column",
            kinds: &[
                "Constraint.conname",
                "Constraint.fk_attrs[0]",
                "Constraint.pk_attrs[0]",
            ],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::Key, n, 9),
        },
        Site {
            id: "pgorm/disjoint.remove-kind.keys.value",
            api: "DisjointShape::remove_kind — the base's foreign keys, a kind's value",
            kinds: &["Constraint.conname", "Constraint.fk_attrs[0]"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind(Slot::Value, n, 9),
        },
        Site {
            id: "pgorm/disjoint.add-kind-table.keys.base",
            api: "DisjointShape::add_kind_table — the generated key columns, keyed NOT VALID, the base",
            kinds: &["Constraint.conname", "AlterTableStmt.relation.relname"],
            policy: Quoted,
            render: |n| disjoint_named::add_kind_table(Slot::Base, n, (0, 1)),
        },
        Site {
            id: "pgorm/disjoint.add-kind-table.keys.value",
            api: "DisjointShape::add_kind_table — the generated key columns, keyed NOT VALID, the value",
            kinds: &[
                "ColumnDef.colname",
                "AConst.val",
                "Constraint.conname",
                "Constraint.fk_attrs[0]",
            ],
            policy: DerivedLabel,
            render: |n| disjoint_named::add_kind_table(Slot::Value, n, (0, 1)),
        },
        Site {
            id: "pgorm/disjoint.add-kind-table.validate.base",
            api: "DisjointShape::add_kind_table — VALIDATE CONSTRAINT, the base",
            kinds: &["AlterTableCmd.name", "AlterTableStmt.relation.relname"],
            policy: Quoted,
            render: |n| disjoint_named::add_kind_table(Slot::Base, n, (1, 0)),
        },
        Site {
            id: "pgorm/disjoint.add-kind-table.validate.key",
            api: "DisjointShape::add_kind_table — VALIDATE CONSTRAINT, the key column",
            kinds: &["AlterTableCmd.name"],
            policy: Quoted,
            render: |n| disjoint_named::add_kind_table(Slot::Key, n, (1, 0)),
        },
        Site {
            id: "pgorm/disjoint.add-kind-table.validate.value",
            api: "DisjointShape::add_kind_table — VALIDATE CONSTRAINT, the value",
            kinds: &["AlterTableCmd.name"],
            policy: Quoted,
            render: |n| disjoint_named::add_kind_table(Slot::Value, n, (1, 0)),
        },
        Site {
            id: "pgorm/disjoint.remove-kind-table.keys.base",
            api: "DisjointShape::remove_kind_table — the generated key columns dropped, the base",
            kinds: &["AlterTableCmd.name", "AlterTableStmt.relation.relname"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind_table(Slot::Base, n, 0),
        },
        Site {
            id: "pgorm/disjoint.remove-kind-table.keys.key",
            api: "DisjointShape::remove_kind_table — the generated key columns dropped, the key column",
            kinds: &["AlterTableCmd.name", "AlterTableCmd.name"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind_table(Slot::Key, n, 0),
        },
        Site {
            id: "pgorm/disjoint.remove-kind-table.keys.value",
            api: "DisjointShape::remove_kind_table — the generated key columns dropped, the value",
            kinds: &["AlterTableCmd.name", "AlterTableCmd.name"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind_table(Slot::Value, n, 0),
        },
        Site {
            id: "pgorm/disjoint.remove-kind-table.drop.table",
            api: "DisjointShape::remove_kind_table — DROP TABLE, the table",
            kinds: &["DropStmt.objects[0].List.items[0]"],
            policy: Quoted,
            render: |n| disjoint_named::remove_kind_table(Slot::Dropped, n, 1),
        },
    ]
}

/// `Schema::create_one_of` over an entity named at run time.
fn one_of(table: &str, schema: Option<&str>) -> Rendered {
    one_of_named::with_names(table, schema, || {
        Rendered::sql(
            Schema::new()
                .create_one_of::<one_of_named::Pinned>()
                .to_string(),
        )
    })
}

fn one_of_sites() -> Vec<Site> {
    vec![
        Site {
            id: "pgorm/one-of.check.table",
            api: "Schema::create_one_of with a runtime EntityName::table_name",
            kinds: &["Constraint.conname", "AlterTableStmt.relation.relname"],
            policy: TableScoped,
            render: |n| one_of(n, None),
        },
        Site {
            id: "pgorm/one-of.check.schema",
            api: "Schema::create_one_of with a runtime EntityName::schema_name",
            kinds: &["AlterTableStmt.relation.schemaname"],
            policy: Quoted,
            render: |n| one_of("t", Some(n)),
        },
    ]
}
