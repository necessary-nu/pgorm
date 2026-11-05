//! The registry, continued: pgorm-query's DDL sites. See `registry.rs` for
//! how to add one.

use pgorm::pgorm_query::{
    Check, ColumnDef, ColumnType, Comment, ConstraintChange, ConstraintDrop, Domain, DomainCheck,
    Enforcement, Expr, ForeignKey, Index, IndexColumn, IndexType, MaterializedView,
    NotNullConstraint, Query, Schema, SelectStatement, Sequence, SequenceOption, Table,
    TableForeignKey, TableKey, TypeName, View,
    extension::{Extension, RangeDefinition, Type},
};
use pgorm_query::ExprTrait;

use super::{
    oracle::{
        Policy::{Literal, Quoted, TypePart},
        Site,
    },
    registry::{fixed, n_, sql},
};

// ---------------------------------------------------------------------------
// pgorm-query: DDL
// ---------------------------------------------------------------------------

pub fn sites() -> Vec<Site> {
    vec![
        // -- CREATE TABLE --------------------------------------------------
        Site {
            id: "ddl/create-table.table",
            api: "Table::create(Name)",
            kinds: &["CreateStmt.relation.relname"],
            policy: Quoted,
            render: |n| sql(Table::create(n_(n)).col(ColumnDef::new(fixed("c")).integer())),
        },
        Site {
            id: "ddl/create-table.schema",
            api: "Table::create((Name, Name)) — the schema part",
            kinds: &["CreateStmt.relation.schemaname"],
            policy: Quoted,
            render: |n| {
                sql(Table::create((n_(n), fixed("t"))).col(ColumnDef::new(fixed("c")).integer()))
            },
        },
        Site {
            id: "ddl/create-table.column",
            api: "ColumnDef::new(Name)",
            kinds: &["ColumnDef.colname"],
            policy: Quoted,
            render: |n| sql(Table::create(fixed("t")).col(ColumnDef::new(n_(n)).integer())),
        },
        Site {
            id: "ddl/create-table.column-not-null-name",
            api: "ColumnDef::not_null_named(Name)",
            kinds: &["Constraint.conname"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer().not_null_named(n_(n))))
            },
        },
        Site {
            id: "ddl/create-table.column-check-name",
            api: "ColumnDef::check(Check::new(..).name(Name))",
            kinds: &["Constraint.conname"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t")).col(
                    ColumnDef::new(fixed("c"))
                        .integer()
                        .check(Check::new(Expr::col(fixed("c")).gt(0)).name(n_(n))),
                ))
            },
        },
        Site {
            id: "ddl/create-table.check-name",
            api: "TableCreateStatement::check(Check::new(..).name(Name))",
            kinds: &["Constraint.conname"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer())
                    .check(
                        Check::new(Expr::col(fixed("c")).gt(0))
                            .name(n_(n))
                            .enforcement(Enforcement::NotEnforced),
                    ))
            },
        },
        Site {
            id: "ddl/create-table.column-type-named",
            api: "ColumnType::named(String)",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new_with_type(fixed("c"), ColumnType::named(n))))
            },
        },
        Site {
            id: "ddl/create-table.column-type-schema",
            api: "ColumnDef::named(TypeName::new(type).schema(Name))",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Table::create(fixed("t")).col(
                    ColumnDef::new(fixed("c")).named(TypeName::new(fixed("ty")).schema(n_(n))),
                ))
            },
        },
        Site {
            id: "ddl/create-table.column-type-raw-schema",
            api: "ColumnDef::named(TypeName::raw(type).schema(Name))",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).named(TypeName::raw("ty").schema(n_(n)))))
            },
        },
        Site {
            id: "ddl/create-table.column-type-array",
            api: "ColumnDef::array(ColumnType::named(String))",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).array(ColumnType::named(n))))
            },
        },
        Site {
            id: "ddl/create-table.column-type-enum",
            api: "ColumnDef::enumeration(Name, labels)",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).enumeration(n_(n), [fixed("a")])))
            },
        },
        Site {
            id: "ddl/create-table.column-type-created-range",
            api: "ColumnType::CreatedRange { name: Name, .. }",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Table::create(fixed("t")).col(ColumnDef::new_with_type(
                    fixed("c"),
                    ColumnType::CreatedRange {
                        name: n_(n),
                        schema: None,
                        subtype: std::sync::Arc::new(ColumnType::Double),
                    },
                )))
            },
        },
        Site {
            id: "ddl/create-table.column-type-created-range-schema",
            api: "ColumnType::CreatedRange { schema: Some(Name), .. }",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Table::create(fixed("t")).col(ColumnDef::new_with_type(
                    fixed("c"),
                    ColumnType::CreatedRange {
                        name: fixed("ty"),
                        schema: Some(n_(n)),
                        subtype: std::sync::Arc::new(ColumnType::Double),
                    },
                )))
            },
        },
        Site {
            id: "ddl/create-table.column-type-created-multirange",
            api: "ColumnType::CreatedMultirange { name: Name, .. }",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Table::create(fixed("t")).col(ColumnDef::new_with_type(
                    fixed("c"),
                    ColumnType::CreatedMultirange {
                        name: n_(n),
                        schema: None,
                        subtype: std::sync::Arc::new(ColumnType::Double),
                    },
                )))
            },
        },
        Site {
            id: "ddl/create-table.column-type-created-multirange-schema",
            api: "ColumnType::CreatedMultirange { schema: Some(Name), .. }",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Table::create(fixed("t")).col(ColumnDef::new_with_type(
                    fixed("c"),
                    ColumnType::CreatedMultirange {
                        name: fixed("ty"),
                        schema: Some(n_(n)),
                        subtype: std::sync::Arc::new(ColumnType::Double),
                    },
                )))
            },
        },
        Site {
            id: "ddl/create-table.column-collation",
            api: "ColumnDef::collate(Name)",
            kinds: &["ColumnDef.coll_clause.collname[0]"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t")).col(ColumnDef::new(fixed("c")).text().collate(n_(n))))
            },
        },
        Site {
            id: "ddl/create-table.column-collation-schema",
            api: "ColumnDef::collate((Name, Name)) — the schema part",
            kinds: &["ColumnDef.coll_clause.collname[0]"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t")).col(
                    ColumnDef::new(fixed("c"))
                        .text()
                        .collate((n_(n), fixed("co"))),
                ))
            },
        },
        Site {
            id: "ddl/create-table.column-collation-qualified",
            api: "ColumnDef::collate((Name, Name)) — the collation part",
            kinds: &["ColumnDef.coll_clause.collname[1]"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t")).col(
                    ColumnDef::new(fixed("c"))
                        .text()
                        .collate((fixed("s"), n_(n))),
                ))
            },
        },
        Site {
            id: "ddl/create-table.key-name",
            api: "TableCreateStatement::unique(TableKey::new(..).name(Name))",
            kinds: &["Constraint.conname"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer())
                    .unique(TableKey::new(fixed("c")).name(n_(n))))
            },
        },
        Site {
            id: "ddl/create-table.unique-key-column",
            api: "TableCreateStatement::unique(Name)",
            kinds: &["Constraint.keys[0]"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer())
                    .unique(n_(n)))
            },
        },
        Site {
            id: "ddl/create-table.key-next-column",
            api: "TableCreateStatement::unique(TableKey::new(..).col(Name))",
            kinds: &["Constraint.keys[1]"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer())
                    .unique(TableKey::new(fixed("c")).col(n_(n))))
            },
        },
        Site {
            id: "ddl/create-table.key-tuple-column",
            api: "TableCreateStatement::unique((.., Name))",
            kinds: &["Constraint.keys[1]"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer())
                    .unique((fixed("c"), n_(n))))
            },
        },
        Site {
            id: "ddl/create-table.nulls-not-distinct-key-column",
            api: "TableCreateStatement::unique(TableKey::new(Name).nulls_not_distinct())",
            kinds: &["Constraint.keys[0]"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer())
                    .unique(TableKey::new(n_(n)).nulls_not_distinct()))
            },
        },
        Site {
            id: "ddl/create-table.primary-key-column",
            api: "TableCreateStatement::primary_key(Name)",
            kinds: &["Constraint.keys[0]"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer())
                    .primary_key(n_(n)))
            },
        },
        Site {
            id: "ddl/create-table.key-include",
            api: "TableCreateStatement::unique(TableKey::new(..).include([Name]))",
            kinds: &["Constraint.including[0]"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer())
                    .unique(TableKey::new(fixed("c")).include([n_(n)])))
            },
        },
        Site {
            id: "ddl/create-table.key-without-overlaps",
            api: "TableCreateStatement::primary_key(TableKey::new(..).without_overlaps(Name))",
            kinds: &["Constraint.keys[1]"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer())
                    .primary_key(TableKey::new(fixed("c")).without_overlaps(n_(n))))
            },
        },
        Site {
            id: "ddl/alter-table.add-unique-without-overlaps",
            api: "TableAlterStatement::add_unique(TableKey::new(..).without_overlaps(Name))",
            kinds: &["Constraint.keys[1]"],
            policy: Quoted,
            render: |n| {
                sql(&Table::alter(fixed("t"))
                    .add_unique(TableKey::new(fixed("c")).without_overlaps(n_(n))))
            },
        },
        Site {
            id: "ddl/alter-table.add-primary-key-column",
            api: "TableAlterStatement::add_primary_key(Name)",
            kinds: &["Constraint.keys[0]"],
            policy: Quoted,
            render: |n| sql(&Table::alter(fixed("t")).add_primary_key(n_(n))),
        },
        Site {
            id: "ddl/alter-table.add-unique-name",
            api: "TableAlterStatement::add_unique(TableKey::new(..).name(Name))",
            kinds: &["Constraint.conname"],
            policy: Quoted,
            render: |n| {
                sql(&Table::alter(fixed("t")).add_unique(TableKey::new(fixed("c")).name(n_(n))))
            },
        },
        Site {
            id: "ddl/create-table.foreign-key-name",
            api: "TableCreateStatement::foreign_key(ForeignKey::create(..).name(Name))",
            kinds: &["Constraint.conname"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer())
                    .foreign_key(
                        ForeignKey::create(fixed("t"), fixed("c"), fixed("r"), fixed("rc"))
                            .name(n_(n))
                            .to_owned(),
                    ))
            },
        },
        Site {
            id: "ddl/create-table.foreign-key-column",
            api: "TableCreateStatement::foreign_key(ForeignKey::create(table, Name, ..))",
            kinds: &["Constraint.fk_attrs[0]"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer())
                    .foreign_key(ForeignKey::create(
                        fixed("t"),
                        n_(n),
                        fixed("r"),
                        fixed("rc"),
                    )))
            },
        },
        Site {
            id: "ddl/create-table.foreign-key-ref-table",
            api: "TableCreateStatement::foreign_key(ForeignKey::create(.., Name, ..))",
            kinds: &["Constraint.pktable.relname"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer())
                    .foreign_key(ForeignKey::create(
                        fixed("t"),
                        fixed("c"),
                        n_(n),
                        fixed("rc"),
                    )))
            },
        },
        Site {
            id: "ddl/create-table.foreign-key-ref-column",
            api: "TableCreateStatement::foreign_key(ForeignKey::create(.., Name))",
            kinds: &["Constraint.pk_attrs[0]"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer())
                    .foreign_key(ForeignKey::create(
                        fixed("t"),
                        fixed("c"),
                        fixed("r"),
                        n_(n),
                    )))
            },
        },
        Site {
            id: "ddl/create-table.foreign-key-period",
            api: "TableCreateStatement::foreign_key(ForeignKey::create(..).period(Name, ..))",
            kinds: &["Constraint.fk_attrs[1]"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer())
                    .foreign_key(
                        ForeignKey::create(fixed("t"), fixed("c"), fixed("r"), fixed("rc"))
                            .period(n_(n), fixed("rp"))
                            .to_owned(),
                    ))
            },
        },
        Site {
            id: "ddl/create-table.foreign-key-ref-period",
            api: "TableCreateStatement::foreign_key(ForeignKey::create(..).period(.., Name))",
            kinds: &["Constraint.pk_attrs[1]"],
            policy: Quoted,
            render: |n| {
                sql(Table::create(fixed("t"))
                    .col(ColumnDef::new(fixed("c")).integer())
                    .foreign_key(
                        ForeignKey::create(fixed("t"), fixed("c"), fixed("r"), fixed("rc"))
                            .period(fixed("p"), n_(n))
                            .to_owned(),
                    ))
            },
        },
        // -- ALTER TABLE ---------------------------------------------------
        Site {
            id: "ddl/alter-table.table",
            api: "Table::alter(Name)",
            kinds: &["AlterTableStmt.relation.relname"],
            policy: Quoted,
            render: |n| sql(&Table::alter(n_(n)).drop_column(fixed("c"))),
        },
        Site {
            id: "ddl/alter-table.add-column",
            api: "TableAlterStatement::add_column(ColumnDef::new(Name))",
            kinds: &["ColumnDef.colname"],
            policy: Quoted,
            render: |n| sql(&Table::alter(fixed("t")).add_column(ColumnDef::new(n_(n)).integer())),
        },
        Site {
            id: "ddl/alter-table.modify-column",
            api: "TableAlterStatement::modify_column(ColumnDef::new(Name))",
            kinds: &["AlterTableCmd.name", "AlterTableCmd.name"],
            policy: Quoted,
            render: |n| {
                sql(&Table::alter(fixed("t"))
                    .modify_column(ColumnDef::new(n_(n)).integer().not_null()))
            },
        },
        Site {
            id: "ddl/alter-table.add-column-collation",
            api: "TableAlterStatement::add_column(ColumnDef::collate(Name))",
            kinds: &["ColumnDef.coll_clause.collname[0]"],
            policy: Quoted,
            render: |n| {
                sql(&Table::alter(fixed("t"))
                    .add_column(ColumnDef::new(fixed("c")).text().collate(n_(n))))
            },
        },
        Site {
            id: "ddl/alter-table.modify-column-collation",
            api: "TableAlterStatement::modify_column(ColumnDef::collate(Name)) — the retype",
            kinds: &["ColumnDef.coll_clause.collname[0]"],
            policy: Quoted,
            render: |n| {
                sql(&Table::alter(fixed("t"))
                    .modify_column(ColumnDef::new(fixed("c")).text().collate(n_(n))))
            },
        },
        Site {
            id: "ddl/alter-table.alter-column-type-using",
            api: "TableAlterStatement::alter_column_type_using(Name, ..)",
            kinds: &["AlterTableCmd.name"],
            policy: Quoted,
            render: |n| {
                sql(&Table::alter(fixed("t")).alter_column_type_using(
                    n_(n),
                    ColumnType::Text,
                    Expr::col(fixed("c")),
                ))
            },
        },
        Site {
            id: "ddl/alter-table.alter-column-type-collated-using",
            api: "TableAlterStatement::alter_column_type_collated_using(.., Name, ..)",
            kinds: &["ColumnDef.coll_clause.collname[0]"],
            policy: Quoted,
            render: |n| {
                sql(&Table::alter(fixed("t")).alter_column_type_collated_using(
                    fixed("c"),
                    ColumnType::Text,
                    n_(n),
                    Expr::col(fixed("c")),
                ))
            },
        },
        Site {
            id: "ddl/alter-table.drop-default",
            api: "TableAlterStatement::drop_default(Name)",
            kinds: &["AlterTableCmd.name"],
            policy: Quoted,
            render: |n| sql(&Table::alter(fixed("t")).drop_default(n_(n))),
        },
        Site {
            id: "ddl/alter-table.drop-column",
            api: "TableAlterStatement::drop_column(Name)",
            kinds: &["AlterTableCmd.name"],
            policy: Quoted,
            render: |n| sql(&Table::alter(fixed("t")).drop_column(n_(n))),
        },
        Site {
            id: "ddl/alter-table.drop-constraint",
            api: "TableAlterStatement::drop_constraint(Name)",
            kinds: &["AlterTableCmd.name"],
            policy: Quoted,
            render: |n| sql(&Table::alter(fixed("t")).drop_constraint(n_(n))),
        },
        Site {
            id: "ddl/alter-table.drop-constraint-if-exists",
            api: "TableAlterStatement::drop_constraint(ConstraintDrop::new(Name).if_exists())",
            kinds: &["AlterTableCmd.name"],
            policy: Quoted,
            render: |n| {
                sql(&Table::alter(fixed("t"))
                    .drop_constraint(ConstraintDrop::new(n_(n)).if_exists().cascade()))
            },
        },
        Site {
            id: "ddl/alter-table.modify-column-not-null-named",
            api: "TableAlterStatement::modify_column(ColumnDef::new(Name).not_null_named(..))",
            kinds: &["Constraint.keys[0]"],
            policy: Quoted,
            render: |n| {
                sql(&Table::alter(fixed("t"))
                    .modify_column(ColumnDef::new(n_(n)).not_null_named(fixed("nn"))))
            },
        },
        Site {
            id: "ddl/alter-table.modify-column-not-null-name",
            api: "TableAlterStatement::modify_column(ColumnDef::not_null_named(Name))",
            kinds: &["Constraint.conname"],
            policy: Quoted,
            render: |n| {
                sql(&Table::alter(fixed("t"))
                    .modify_column(ColumnDef::new(fixed("c")).not_null_named(n_(n))))
            },
        },
        Site {
            id: "ddl/alter-table.add-not-null-column",
            api: "TableAlterStatement::add_not_null(NotNullConstraint::new(Name))",
            kinds: &["Constraint.keys[0]"],
            policy: Quoted,
            render: |n| sql(&Table::alter(fixed("t")).add_not_null(NotNullConstraint::new(n_(n)))),
        },
        Site {
            id: "ddl/alter-table.add-not-null-name",
            api: "TableAlterStatement::add_not_null(NotNullConstraint::new(..).name(Name))",
            kinds: &["Constraint.conname"],
            policy: Quoted,
            render: |n| {
                sql(&Table::alter(fixed("t"))
                    .add_not_null(NotNullConstraint::new(fixed("c")).name(n_(n)).not_valid()))
            },
        },
        Site {
            id: "ddl/alter-table.add-check-name",
            api: "TableAlterStatement::add_check(Check::new(..).name(Name))",
            kinds: &["Constraint.conname"],
            policy: Quoted,
            render: |n| {
                sql(&Table::alter(fixed("t"))
                    .add_check(Check::new(Expr::col(fixed("c")).gt(0)).name(n_(n))))
            },
        },
        Site {
            id: "ddl/alter-table.validate-constraint",
            api: "TableAlterStatement::validate_constraint(Name)",
            kinds: &["AlterTableCmd.name"],
            policy: Quoted,
            render: |n| sql(&Table::alter(fixed("t")).validate_constraint(n_(n))),
        },
        Site {
            id: "ddl/alter-table.alter-constraint",
            api: "TableAlterStatement::alter_constraint(Name, ..)",
            kinds: &["AtalterConstraint.conname"],
            policy: Quoted,
            render: |n| {
                sql(&Table::alter(fixed("t")).alter_constraint(n_(n), ConstraintChange::NoInherit))
            },
        },
        Site {
            id: "ddl/alter-table.add-foreign-key-name",
            api: "TableAlterStatement::add_foreign_key(TableForeignKey::new(..).name(Name))",
            kinds: &["Constraint.conname"],
            policy: Quoted,
            render: |n| {
                sql(&Table::alter(fixed("t")).add_foreign_key(
                    TableForeignKey::new(fixed("t"), fixed("c"), fixed("r"), fixed("rc"))
                        .name(n_(n))
                        .to_owned(),
                ))
            },
        },
        Site {
            id: "ddl/alter-table.add-foreign-key-period",
            api: "TableAlterStatement::add_foreign_key(TableForeignKey::new(..).period(Name, ..))",
            kinds: &["Constraint.fk_attrs[1]"],
            policy: Quoted,
            render: |n| {
                sql(&Table::alter(fixed("t")).add_foreign_key(
                    TableForeignKey::new(fixed("t"), fixed("c"), fixed("r"), fixed("rc"))
                        .period(n_(n), fixed("rp"))
                        .to_owned(),
                ))
            },
        },
        Site {
            id: "ddl/rename-table.to",
            api: "Table::rename(table, Name)",
            kinds: &["RenameStmt.newname"],
            policy: Quoted,
            render: |n| sql(&Table::rename(fixed("t"), n_(n))),
        },
        Site {
            id: "ddl/rename-table.from",
            api: "Table::rename(Name, to)",
            kinds: &["RenameStmt.relation.relname"],
            policy: Quoted,
            render: |n| sql(&Table::rename(n_(n), fixed("u"))),
        },
        Site {
            id: "ddl/rename-column.from",
            api: "Table::rename_column(table, Name, to)",
            kinds: &["RenameStmt.subname"],
            policy: Quoted,
            render: |n| sql(&Table::rename_column(fixed("t"), n_(n), fixed("d"))),
        },
        Site {
            id: "ddl/rename-column.to",
            api: "Table::rename_column(table, from, Name)",
            kinds: &["RenameStmt.newname"],
            policy: Quoted,
            render: |n| sql(&Table::rename_column(fixed("t"), fixed("c"), n_(n))),
        },
        Site {
            id: "ddl/rename-constraint.table",
            api: "Table::rename_constraint(Name, from, to)",
            kinds: &["RenameStmt.relation.relname"],
            policy: Quoted,
            render: |n| sql(&Table::rename_constraint(n_(n), fixed("k"), fixed("l"))),
        },
        Site {
            id: "ddl/rename-constraint.from",
            api: "Table::rename_constraint(table, Name, to)",
            kinds: &["RenameStmt.subname"],
            policy: Quoted,
            render: |n| sql(&Table::rename_constraint(fixed("t"), n_(n), fixed("l"))),
        },
        Site {
            id: "ddl/rename-constraint.to",
            api: "Table::rename_constraint(table, from, Name)",
            kinds: &["RenameStmt.newname"],
            policy: Quoted,
            render: |n| sql(&Table::rename_constraint(fixed("t"), fixed("k"), n_(n))),
        },
        Site {
            id: "ddl/drop-table.table",
            api: "Table::drop(Name)",
            kinds: &["DropStmt.objects[0].List.items[0]"],
            policy: Quoted,
            render: |n| sql(&Table::drop(n_(n))),
        },
        Site {
            id: "ddl/truncate-table.table",
            api: "Table::truncate(Name)",
            kinds: &["RangeVar.relname"],
            policy: Quoted,
            render: |n| sql(&Table::truncate(n_(n))),
        },
        // -- COMMENT ON ----------------------------------------------------
        Site {
            id: "ddl/comment.table",
            api: "Comment::on_table(Name, text)",
            kinds: &["CommentStmt.object.List.items[0]"],
            policy: Quoted,
            render: |n| sql(&Comment::on_table(n_(n), "x")),
        },
        Site {
            id: "ddl/comment.column",
            api: "Comment::on_column(table, Name, text)",
            kinds: &["CommentStmt.object.List.items[1]"],
            policy: Quoted,
            render: |n| sql(&Comment::on_column(fixed("t"), n_(n), "x")),
        },
        // -- CREATE / DROP INDEX -------------------------------------------
        Site {
            id: "ddl/create-index.name",
            api: "IndexCreateStatement::name(Name)",
            kinds: &["IndexStmt.idxname"],
            policy: Quoted,
            render: |n| sql(Index::create(fixed("t"), fixed("c")).name(n_(n))),
        },
        Site {
            id: "ddl/create-index.table",
            api: "Index::create(Name, column)",
            kinds: &["IndexStmt.relation.relname"],
            policy: Quoted,
            render: |n| sql(Index::create(n_(n), fixed("c")).name(fixed("i"))),
        },
        Site {
            id: "ddl/create-index.column",
            api: "Index::create(table, Name)",
            kinds: &["IndexElem.name"],
            policy: Quoted,
            render: |n| sql(Index::create(fixed("t"), n_(n)).name(fixed("i"))),
        },
        Site {
            id: "ddl/create-index.operator-class",
            api: "IndexColumn::operator_class(Name)",
            kinds: &["IndexElem.opclass[0]"],
            policy: Quoted,
            render: |n| {
                sql(Index::create(
                    fixed("t"),
                    IndexColumn::name(fixed("c")).operator_class(n_(n)),
                )
                .name(fixed("i")))
            },
        },
        Site {
            id: "ddl/create-index.access-method",
            api: "IndexCreateStatement::index_type(IndexType::Named(Name))",
            kinds: &["IndexStmt.access_method"],
            policy: TypePart,
            render: |n| {
                sql(Index::create(fixed("t"), fixed("c"))
                    .name(fixed("i"))
                    .index_type(IndexType::Named(n_(n))))
            },
        },
        Site {
            id: "ddl/create-index.include",
            api: "IndexCreateStatement::include([Name])",
            kinds: &["IndexElem.name"],
            policy: Quoted,
            render: |n| {
                sql(Index::create(fixed("t"), fixed("c"))
                    .name(fixed("i"))
                    .include([n_(n)]))
            },
        },
        Site {
            id: "ddl/drop-index.name",
            api: "Index::drop(Name)",
            kinds: &["DropStmt.objects[0].List.items[0]"],
            policy: Quoted,
            render: |n| sql(&Index::drop(n_(n))),
        },
        Site {
            id: "ddl/drop-index.schema",
            api: "IndexDropStatement::table((Name, table)) — the schema part",
            kinds: &["DropStmt.objects[0].List.items[0]"],
            policy: Quoted,
            render: |n| sql(Index::drop(fixed("i")).table((n_(n), fixed("t")))),
        },
        // -- FOREIGN KEY ---------------------------------------------------
        Site {
            id: "ddl/foreign-key.name",
            api: "ForeignKey::create(..).name(Name)",
            kinds: &["Constraint.conname"],
            policy: Quoted,
            render: |n| {
                sql(ForeignKey::create(fixed("t"), fixed("c"), fixed("r"), fixed("rc")).name(n_(n)))
            },
        },
        Site {
            id: "ddl/foreign-key.table",
            api: "ForeignKey::create(Name, ..)",
            kinds: &["AlterTableStmt.relation.relname"],
            policy: Quoted,
            render: |n| {
                sql(&ForeignKey::create(
                    n_(n),
                    fixed("c"),
                    fixed("r"),
                    fixed("rc"),
                ))
            },
        },
        Site {
            id: "ddl/foreign-key.column",
            api: "ForeignKey::create(table, Name, ..)",
            kinds: &["Constraint.fk_attrs[0]"],
            policy: Quoted,
            render: |n| {
                sql(&ForeignKey::create(
                    fixed("t"),
                    n_(n),
                    fixed("r"),
                    fixed("rc"),
                ))
            },
        },
        Site {
            id: "ddl/foreign-key.ref-table",
            api: "ForeignKey::create(table, column, Name, ..)",
            kinds: &["Constraint.pktable.relname"],
            policy: Quoted,
            render: |n| {
                sql(&ForeignKey::create(
                    fixed("t"),
                    fixed("c"),
                    n_(n),
                    fixed("rc"),
                ))
            },
        },
        Site {
            id: "ddl/foreign-key.ref-column",
            api: "ForeignKey::create(table, column, ref_table, Name)",
            kinds: &["Constraint.pk_attrs[0]"],
            policy: Quoted,
            render: |n| {
                sql(&ForeignKey::create(
                    fixed("t"),
                    fixed("c"),
                    fixed("r"),
                    n_(n),
                ))
            },
        },
        // -- TYPE ----------------------------------------------------------
        Site {
            id: "ddl/create-type.name",
            api: "Type::create(Name)",
            kinds: &["CreateEnumStmt.type_name[0]"],
            policy: Quoted,
            render: |n| sql(Type::create(n_(n)).as_enum().values(["a"])),
        },
        Site {
            id: "ddl/create-type.schema",
            api: "Type::create((Name, type)) — the schema part",
            kinds: &["CreateEnumStmt.type_name[0]"],
            policy: Quoted,
            render: |n| sql(Type::create((n_(n), fixed("ty"))).as_enum().values(["a"])),
        },
        Site {
            id: "ddl/create-type.database",
            api: "Type::create((Name, schema, type)) — the database part",
            kinds: &["CreateEnumStmt.type_name[0]"],
            policy: Quoted,
            render: |n| {
                sql(Type::create((n_(n), fixed("s"), fixed("ty")))
                    .as_enum()
                    .values(["a"]))
            },
        },
        Site {
            id: "ddl/create-type.label",
            api: "TypeCreateStatement::values([String]) — enum labels are values",
            kinds: &["CreateEnumStmt.vals[0]"],
            policy: Literal,
            render: |n| sql(Type::create(fixed("ty")).as_enum().values([n])),
        },
        Site {
            id: "ddl/create-type.composite-name",
            api: "Type::create(Name).attribute(..)",
            kinds: &["CompositeTypeStmt.typevar.relname"],
            policy: Quoted,
            render: |n| sql(Type::create(n_(n)).attribute(fixed("a"), ColumnType::Integer)),
        },
        Site {
            id: "ddl/create-type.composite-schema",
            api: "Type::create((Name, type)).attribute(..) — the schema part",
            kinds: &["CompositeTypeStmt.typevar.schemaname"],
            policy: Quoted,
            render: |n| {
                sql(Type::create((n_(n), fixed("ty"))).attribute(fixed("a"), ColumnType::Integer))
            },
        },
        Site {
            id: "ddl/create-type.composite-database",
            api: "Type::create((Name, schema, type)).attribute(..) — the database part",
            kinds: &["CompositeTypeStmt.typevar.catalogname"],
            policy: Quoted,
            render: |n| {
                sql(Type::create((n_(n), fixed("s"), fixed("ty")))
                    .attribute(fixed("a"), ColumnType::Integer))
            },
        },
        Site {
            id: "ddl/create-type.attribute",
            api: "TypeCreateStatement::attribute(Name, type)",
            kinds: &["ColumnDef.colname"],
            policy: Quoted,
            render: |n| sql(Type::create(fixed("ty")).attribute(n_(n), ColumnType::Integer)),
        },
        Site {
            id: "ddl/create-type.attribute-type",
            api: "TypeCreateStatement::attribute(name, ColumnType::named(String))",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| sql(Type::create(fixed("ty")).attribute(fixed("a"), ColumnType::named(n))),
        },
        Site {
            id: "ddl/create-type.attribute-collation",
            api: "TypeCreateStatement::attribute_collated(name, type, Name)",
            kinds: &["ColumnDef.coll_clause.collname[0]"],
            policy: Quoted,
            render: |n| {
                sql(Type::create(fixed("ty")).attribute_collated(
                    fixed("a"),
                    ColumnType::Text,
                    n_(n),
                ))
            },
        },
        Site {
            id: "ddl/create-type.attribute-collation-qualified",
            api: "TypeCreateStatement::attribute_collated(name, type, (Name, Name)) — the collation part",
            kinds: &["ColumnDef.coll_clause.collname[1]"],
            policy: Quoted,
            render: |n| {
                sql(Type::create(fixed("ty")).attribute_collated(
                    fixed("a"),
                    ColumnType::Text,
                    (fixed("s"), n_(n)),
                ))
            },
        },
        Site {
            id: "ddl/create-type.range-name",
            api: "Type::create(Name).as_range(..)",
            kinds: &["CreateRangeStmt.type_name[0]"],
            policy: Quoted,
            render: |n| sql(Type::create(n_(n)).as_range(RangeDefinition::new(ColumnType::Integer))),
        },
        Site {
            id: "ddl/create-type.range-schema",
            api: "Type::create((Name, type)).as_range(..) — the schema part",
            kinds: &["CreateRangeStmt.type_name[0]"],
            policy: Quoted,
            render: |n| {
                sql(Type::create((n_(n), fixed("ty")))
                    .as_range(RangeDefinition::new(ColumnType::Integer)))
            },
        },
        Site {
            id: "ddl/create-type.range-database",
            api: "Type::create((Name, schema, type)).as_range(..) — the database part",
            kinds: &["CreateRangeStmt.type_name[0]"],
            policy: Quoted,
            render: |n| {
                sql(Type::create((n_(n), fixed("s"), fixed("ty")))
                    .as_range(RangeDefinition::new(ColumnType::Integer)))
            },
        },
        Site {
            id: "ddl/create-type.range-subtype",
            api: "RangeDefinition::new(ColumnType::named(String))",
            kinds: &["TypeName.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Type::create(fixed("ty")).as_range(RangeDefinition::new(ColumnType::named(n))))
            },
        },
        Site {
            id: "ddl/create-type.range-opclass",
            api: "RangeDefinition::subtype_opclass(Name)",
            kinds: &["TypeName.names[0]"],
            policy: Quoted,
            render: |n| {
                sql(Type::create(fixed("ty"))
                    .as_range(RangeDefinition::new(ColumnType::Integer).subtype_opclass(n_(n))))
            },
        },
        Site {
            id: "ddl/create-type.range-collation",
            api: "RangeDefinition::collation(Name)",
            kinds: &["TypeName.names[0]"],
            policy: Quoted,
            render: |n| {
                sql(Type::create(fixed("ty"))
                    .as_range(RangeDefinition::new(ColumnType::Text).collation(n_(n))))
            },
        },
        Site {
            id: "ddl/create-type.range-collation-qualified",
            api: "RangeDefinition::collation((Name, Name)) — the collation part",
            kinds: &["TypeName.names[1]"],
            policy: Quoted,
            render: |n| {
                sql(Type::create(fixed("ty")).as_range(
                    RangeDefinition::new(ColumnType::Text).collation((fixed("s"), n_(n))),
                ))
            },
        },
        Site {
            id: "ddl/create-type.range-subtype-diff",
            api: "RangeDefinition::subtype_diff(Name)",
            kinds: &["TypeName.names[0]"],
            policy: Quoted,
            render: |n| {
                sql(Type::create(fixed("ty"))
                    .as_range(RangeDefinition::new(ColumnType::Double).subtype_diff(n_(n))))
            },
        },
        Site {
            id: "ddl/create-type.multirange-name",
            api: "RangeDefinition::multirange_type_name(Name)",
            kinds: &["TypeName.names[0]"],
            policy: Quoted,
            render: |n| {
                sql(Type::create(fixed("ty")).as_range(
                    RangeDefinition::new(ColumnType::Integer).multirange_type_name(n_(n)),
                ))
            },
        },
        Site {
            id: "ddl/create-type.multirange-schema",
            api: "RangeDefinition::multirange_type_name((Name, type)) — the schema part",
            kinds: &["TypeName.names[0]"],
            policy: Quoted,
            render: |n| {
                sql(Type::create(fixed("ty")).as_range(
                    RangeDefinition::new(ColumnType::Integer)
                        .multirange_type_name((n_(n), fixed("tys"))),
                ))
            },
        },
        Site {
            id: "ddl/create-type.multirange-database",
            api: "RangeDefinition::multirange_type_name((Name, schema, type)) — the database part",
            kinds: &["TypeName.names[0]"],
            policy: Quoted,
            render: |n| {
                sql(Type::create(fixed("ty")).as_range(
                    RangeDefinition::new(ColumnType::Integer).multirange_type_name((
                        n_(n),
                        fixed("s"),
                        fixed("tys"),
                    )),
                ))
            },
        },
        Site {
            id: "ddl/drop-type.name",
            api: "Type::drop(Name)",
            kinds: &["TypeName.names[0]"],
            policy: Quoted,
            render: |n| sql(&Type::drop(n_(n))),
        },
        Site {
            id: "ddl/alter-type.name",
            api: "Type::alter(Name)",
            kinds: &["AlterEnumStmt.type_name[0]"],
            policy: Quoted,
            render: |n| sql(&Type::alter(n_(n)).add_value("a")),
        },
        Site {
            id: "ddl/alter-type.rename-to",
            api: "PendingTypeAlter::rename_to(Name)",
            kinds: &["RenameStmt.newname"],
            policy: Quoted,
            render: |n| sql(&Type::alter(fixed("ty")).rename_to(n_(n))),
        },
        Site {
            id: "ddl/alter-type.add-value",
            api: "PendingTypeAlter::add_value(String) — a label, a value",
            kinds: &["AlterEnumStmt.new_val"],
            policy: Literal,
            render: |n| sql(&Type::alter(fixed("ty")).add_value(n)),
        },
        Site {
            id: "ddl/alter-type.add-value-before",
            api: "TypeAlterStatement::before(String) — a label, a value",
            kinds: &["AlterEnumStmt.new_val_neighbor"],
            policy: Literal,
            render: |n| sql(&Type::alter(fixed("ty")).add_value("a").before(n)),
        },
        Site {
            id: "ddl/alter-type.rename-value-existing",
            api: "PendingTypeAlter::rename_value(String, new) — a label, a value",
            kinds: &["AlterEnumStmt.old_val"],
            policy: Literal,
            render: |n| sql(&Type::alter(fixed("ty")).rename_value(n, "b")),
        },
        Site {
            id: "ddl/alter-type.rename-value-new",
            api: "PendingTypeAlter::rename_value(existing, String) — a label, a value",
            kinds: &["AlterEnumStmt.new_val"],
            policy: Literal,
            render: |n| sql(&Type::alter(fixed("ty")).rename_value("a", n)),
        },
        Site {
            id: "ddl/alter-type.composite-name",
            api: "Type::alter(Name).add_attribute(..)",
            kinds: &["AlterTableStmt.relation.relname"],
            policy: Quoted,
            render: |n| sql(&Type::alter(n_(n)).add_attribute(fixed("a"), ColumnType::Integer)),
        },
        Site {
            id: "ddl/alter-type.add-attribute",
            api: "PendingTypeAlter::add_attribute(Name, type)",
            kinds: &["ColumnDef.colname"],
            policy: Quoted,
            render: |n| sql(&Type::alter(fixed("ty")).add_attribute(n_(n), ColumnType::Integer)),
        },
        Site {
            id: "ddl/alter-type.add-attribute-type",
            api: "PendingTypeAlter::add_attribute(name, ColumnType::named(String))",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(&Type::alter(fixed("ty")).add_attribute(fixed("a"), ColumnType::named(n)))
            },
        },
        Site {
            id: "ddl/alter-type.add-attribute-collation",
            api: "PendingTypeAlter::add_attribute_collated(name, type, Name)",
            kinds: &["ColumnDef.coll_clause.collname[0]"],
            policy: Quoted,
            render: |n| {
                sql(&Type::alter(fixed("ty")).add_attribute_collated(
                    fixed("a"),
                    ColumnType::Text,
                    n_(n),
                ))
            },
        },
        Site {
            id: "ddl/alter-type.drop-attribute",
            api: "PendingTypeAlter::drop_attribute(Name)",
            kinds: &["AlterTableCmd.name"],
            policy: Quoted,
            render: |n| sql(&Type::alter(fixed("ty")).drop_attribute(n_(n))),
        },
        Site {
            id: "ddl/alter-type.drop-attribute-if-exists",
            api: "CompositeAlterStatement::drop_attribute_if_exists(Name)",
            kinds: &["AlterTableCmd.name"],
            policy: Quoted,
            render: |n| {
                sql(&Type::alter(fixed("ty"))
                    .drop_attribute(fixed("a"))
                    .drop_attribute_if_exists(n_(n)))
            },
        },
        Site {
            id: "ddl/alter-type.alter-attribute",
            api: "PendingTypeAlter::alter_attribute(Name, type)",
            kinds: &["AlterTableCmd.name"],
            policy: Quoted,
            render: |n| {
                sql(&Type::alter(fixed("ty")).alter_attribute(n_(n), ColumnType::BigInteger))
            },
        },
        Site {
            id: "ddl/alter-type.alter-attribute-collation",
            api: "PendingTypeAlter::alter_attribute_collated(name, type, Name)",
            kinds: &["ColumnDef.coll_clause.collname[0]"],
            policy: Quoted,
            render: |n| {
                sql(&Type::alter(fixed("ty")).alter_attribute_collated(
                    fixed("a"),
                    ColumnType::Text,
                    n_(n),
                ))
            },
        },
        Site {
            id: "ddl/alter-type.rename-attribute-type",
            api: "Type::alter(Name).rename_attribute(..)",
            kinds: &["RenameStmt.relation.relname"],
            policy: Quoted,
            render: |n| sql(&Type::alter(n_(n)).rename_attribute(fixed("a"), fixed("b"))),
        },
        Site {
            id: "ddl/alter-type.rename-attribute-from",
            api: "PendingTypeAlter::rename_attribute(Name, to)",
            kinds: &["RenameStmt.subname"],
            policy: Quoted,
            render: |n| sql(&Type::alter(fixed("ty")).rename_attribute(n_(n), fixed("b"))),
        },
        Site {
            id: "ddl/alter-type.rename-attribute-to",
            api: "PendingTypeAlter::rename_attribute(from, Name)",
            kinds: &["RenameStmt.newname"],
            policy: Quoted,
            render: |n| sql(&Type::alter(fixed("ty")).rename_attribute(fixed("a"), n_(n))),
        },
        // -- SEQUENCE ------------------------------------------------------
        Site {
            id: "ddl/create-sequence.name",
            api: "Sequence::create(Name)",
            kinds: &["CreateSeqStmt.sequence.relname"],
            policy: Quoted,
            render: |n| sql(&Sequence::create(n_(n))),
        },
        Site {
            id: "ddl/create-sequence.schema",
            api: "Sequence::create((Name, Name)) — the schema part",
            kinds: &["CreateSeqStmt.sequence.schemaname"],
            policy: Quoted,
            render: |n| sql(&Sequence::create((n_(n), fixed("s")))),
        },
        Site {
            id: "ddl/create-sequence.owned-by-table",
            api: "SequenceCreateStatement::owned_by(Name, column) — the table",
            kinds: &["DefElem.arg.List.items[0]"],
            policy: Quoted,
            render: |n| sql(Sequence::create(fixed("s")).owned_by(n_(n), fixed("c"))),
        },
        Site {
            id: "ddl/create-sequence.owned-by-schema",
            api: "SequenceCreateStatement::owned_by((Name, Name), column) — the schema part",
            kinds: &["DefElem.arg.List.items[0]"],
            policy: Quoted,
            render: |n| sql(Sequence::create(fixed("s")).owned_by((n_(n), fixed("t")), fixed("c"))),
        },
        Site {
            id: "ddl/create-sequence.owned-by-column",
            api: "SequenceCreateStatement::owned_by(table, Name) — the column",
            kinds: &["DefElem.arg.List.items[1]"],
            policy: Quoted,
            render: |n| sql(Sequence::create(fixed("s")).owned_by(fixed("t"), n_(n))),
        },
        Site {
            id: "ddl/alter-sequence.name",
            api: "Sequence::alter(Name)",
            kinds: &["AlterSeqStmt.sequence.relname"],
            policy: Quoted,
            render: |n| sql(&Sequence::alter(n_(n)).restart()),
        },
        Site {
            id: "ddl/alter-sequence.schema",
            api: "Sequence::alter((Name, Name)) — the schema part",
            kinds: &["AlterSeqStmt.sequence.schemaname"],
            policy: Quoted,
            render: |n| sql(&Sequence::alter((n_(n), fixed("s"))).options(SequenceOption::Cache(2))),
        },
        Site {
            id: "ddl/alter-sequence.owned-by-table",
            api: "PendingSequenceAlter::owned_by(Name, column) — the table",
            kinds: &["DefElem.arg.List.items[0]"],
            policy: Quoted,
            render: |n| sql(&Sequence::alter(fixed("s")).owned_by(n_(n), fixed("c"))),
        },
        Site {
            id: "ddl/alter-sequence.owned-by-column",
            api: "SequenceAlterStatement::owned_by(table, Name) — the column",
            kinds: &["DefElem.arg.List.items[1]"],
            policy: Quoted,
            render: |n| {
                sql(Sequence::alter(fixed("s"))
                    .restart()
                    .owned_by(fixed("t"), n_(n)))
            },
        },
        Site {
            id: "ddl/drop-sequence.name",
            api: "Sequence::drop(Name)",
            kinds: &["DropStmt.objects[0].List.items[0]"],
            policy: Quoted,
            render: |n| sql(&Sequence::drop(n_(n))),
        },
        Site {
            id: "ddl/drop-sequence.further-name",
            api: "SequenceDropStatement::name(Name)",
            kinds: &["DropStmt.objects[1].List.items[0]"],
            policy: Quoted,
            render: |n| sql(Sequence::drop(fixed("s")).name(n_(n))),
        },
        Site {
            id: "ddl/drop-sequence.schema",
            api: "Sequence::drop((Name, Name)) — the schema part",
            kinds: &["DropStmt.objects[0].List.items[0]"],
            policy: Quoted,
            render: |n| sql(&Sequence::drop((n_(n), fixed("s")))),
        },
        Site {
            id: "ddl/rename-sequence.from",
            api: "Sequence::rename(Name, to)",
            kinds: &["RenameStmt.relation.relname"],
            policy: Quoted,
            render: |n| sql(&Sequence::rename(n_(n), fixed("t"))),
        },
        Site {
            id: "ddl/rename-sequence.schema",
            api: "Sequence::rename((Name, Name), to) — the schema part",
            kinds: &["RenameStmt.relation.schemaname"],
            policy: Quoted,
            render: |n| sql(&Sequence::rename((n_(n), fixed("s")), fixed("t"))),
        },
        Site {
            id: "ddl/rename-sequence.to",
            api: "Sequence::rename(from, Name)",
            kinds: &["RenameStmt.newname"],
            policy: Quoted,
            render: |n| sql(&Sequence::rename(fixed("s"), n_(n))),
        },
        // -- VIEW ----------------------------------------------------------
        Site {
            id: "ddl/create-view.name",
            api: "View::create(Name, body)",
            kinds: &["ViewStmt.view.relname"],
            policy: Quoted,
            render: |n| sql(&View::create(n_(n), body()).expect(BODY_TAKEN)),
        },
        Site {
            id: "ddl/create-view.schema",
            api: "View::create((Name, Name), body) — the schema part",
            kinds: &["ViewStmt.view.schemaname"],
            policy: Quoted,
            render: |n| sql(&View::create((n_(n), fixed("v")), body()).expect(BODY_TAKEN)),
        },
        Site {
            id: "ddl/create-view.column",
            api: "ViewCreateStatement::column(Name)",
            kinds: &["ViewStmt.aliases[0]"],
            policy: Quoted,
            render: |n| {
                sql(View::create(fixed("v"), body())
                    .expect(BODY_TAKEN)
                    .column(n_(n)))
            },
        },
        Site {
            id: "ddl/create-view.columns",
            api: "ViewCreateStatement::columns([Name])",
            kinds: &["ViewStmt.aliases[1]"],
            policy: Quoted,
            render: |n| {
                sql(View::create(fixed("v"), body())
                    .expect(BODY_TAKEN)
                    .columns([fixed("a"), n_(n)]))
            },
        },
        Site {
            id: "ddl/create-recursive-view.name",
            api: "View::create_recursive(Name, column, body)",
            kinds: &[
                "RangeVar.relname",
                "CommonTableExpr.ctename",
                "ViewStmt.view.relname",
            ],
            policy: Quoted,
            render: |n| sql(&View::create_recursive(n_(n), fixed("c"), body()).expect(BODY_TAKEN)),
        },
        Site {
            id: "ddl/create-recursive-view.first-column",
            api: "View::create_recursive(name, Name, body)",
            kinds: &[
                "ViewStmt.aliases[0]",
                "ColumnRef.fields[0]",
                "CommonTableExpr.aliascolnames[0]",
            ],
            policy: Quoted,
            render: |n| sql(&View::create_recursive(fixed("v"), n_(n), body()).expect(BODY_TAKEN)),
        },
        Site {
            id: "ddl/create-recursive-view.column",
            api: "RecursiveViewCreateStatement::column(Name)",
            kinds: &[
                "ViewStmt.aliases[1]",
                "ColumnRef.fields[0]",
                "CommonTableExpr.aliascolnames[1]",
            ],
            policy: Quoted,
            render: |n| {
                sql(View::create_recursive(fixed("v"), fixed("c"), body())
                    .expect(BODY_TAKEN)
                    .column(n_(n)))
            },
        },
        Site {
            id: "ddl/drop-view.name",
            api: "View::drop(Name)",
            kinds: &["DropStmt.objects[0].List.items[0]"],
            policy: Quoted,
            render: |n| sql(&View::drop(n_(n))),
        },
        Site {
            id: "ddl/drop-view.further-name",
            api: "ViewDropStatement::name(Name)",
            kinds: &["DropStmt.objects[1].List.items[0]"],
            policy: Quoted,
            render: |n| sql(View::drop(fixed("v")).name(n_(n))),
        },
        Site {
            id: "ddl/drop-view.schema",
            api: "View::drop((Name, Name)) — the schema part",
            kinds: &["DropStmt.objects[0].List.items[0]"],
            policy: Quoted,
            render: |n| sql(&View::drop((n_(n), fixed("v")))),
        },
        // -- MATERIALIZED VIEW ---------------------------------------------
        Site {
            id: "ddl/create-materialized-view.name",
            api: "MaterializedView::create(Name, body)",
            kinds: &["CreateTableAsStmt.into.rel.relname"],
            policy: Quoted,
            render: |n| sql(&MaterializedView::create(n_(n), body()).expect(BODY_TAKEN)),
        },
        Site {
            id: "ddl/create-materialized-view.schema",
            api: "MaterializedView::create((Name, Name), body) — the schema part",
            kinds: &["CreateTableAsStmt.into.rel.schemaname"],
            policy: Quoted,
            render: |n| {
                sql(&MaterializedView::create((n_(n), fixed("m")), body()).expect(BODY_TAKEN))
            },
        },
        Site {
            id: "ddl/create-materialized-view.column",
            api: "MaterializedViewCreateStatement::column(Name)",
            kinds: &["CreateTableAsStmt.into.col_names[0]"],
            policy: Quoted,
            render: |n| {
                sql(MaterializedView::create(fixed("m"), body())
                    .expect(BODY_TAKEN)
                    .column(n_(n)))
            },
        },
        Site {
            id: "ddl/refresh-materialized-view.name",
            api: "MaterializedView::refresh(Name)",
            kinds: &["RefreshMatViewStmt.relation.relname"],
            policy: Quoted,
            render: |n| sql(&MaterializedView::refresh(n_(n))),
        },
        Site {
            id: "ddl/refresh-materialized-view.schema",
            api: "MaterializedView::refresh((Name, Name)) — the schema part",
            kinds: &["RefreshMatViewStmt.relation.schemaname"],
            policy: Quoted,
            render: |n| sql(&MaterializedView::refresh((n_(n), fixed("m")))),
        },
        Site {
            id: "ddl/drop-materialized-view.name",
            api: "MaterializedView::drop(Name)",
            kinds: &["DropStmt.objects[0].List.items[0]"],
            policy: Quoted,
            render: |n| sql(&MaterializedView::drop(n_(n))),
        },
        Site {
            id: "ddl/drop-materialized-view.further-name",
            api: "MaterializedViewDropStatement::name(Name)",
            kinds: &["DropStmt.objects[1].List.items[0]"],
            policy: Quoted,
            render: |n| sql(MaterializedView::drop(fixed("m")).name(n_(n))),
        },
        // -- CREATE TABLE AS -----------------------------------------------
        Site {
            id: "ddl/create-table-as.name",
            api: "Table::create_as(Name, body)",
            kinds: &["CreateTableAsStmt.into.rel.relname"],
            policy: Quoted,
            render: |n| sql(&Table::create_as(n_(n), body()).expect(BODY_TAKEN)),
        },
        Site {
            id: "ddl/create-table-as.schema",
            api: "Table::create_as((Name, Name), body) — the schema part",
            kinds: &["CreateTableAsStmt.into.rel.schemaname"],
            policy: Quoted,
            render: |n| sql(&Table::create_as((n_(n), fixed("t")), body()).expect(BODY_TAKEN)),
        },
        Site {
            id: "ddl/create-table-as.column",
            api: "TableCreateAsStatement::column(Name)",
            kinds: &["CreateTableAsStmt.into.col_names[0]"],
            policy: Quoted,
            render: |n| {
                sql(Table::create_as(fixed("t"), body())
                    .expect(BODY_TAKEN)
                    .column(n_(n)))
            },
        },
        // -- SCHEMA --------------------------------------------------------
        Site {
            id: "ddl/create-schema.name",
            api: "Schema::create(Name)",
            kinds: &["CreateSchemaStmt.schemaname"],
            policy: Quoted,
            render: |n| sql(&Schema::create(n_(n))),
        },
        Site {
            id: "ddl/drop-schema.name",
            api: "Schema::drop(Name)",
            kinds: &["DropStmt.objects[0]"],
            policy: Quoted,
            render: |n| sql(&Schema::drop(n_(n))),
        },
        Site {
            id: "ddl/drop-schema.further-name",
            api: "SchemaDropStatement::name(Name)",
            kinds: &["DropStmt.objects[1]"],
            policy: Quoted,
            render: |n| sql(Schema::drop(fixed("s")).name(n_(n))),
        },
        Site {
            id: "ddl/rename-schema.from",
            api: "Schema::rename(Name, to)",
            kinds: &["RenameStmt.subname"],
            policy: Quoted,
            render: |n| sql(&Schema::rename(n_(n), fixed("s"))),
        },
        Site {
            id: "ddl/rename-schema.to",
            api: "Schema::rename(from, Name)",
            kinds: &["RenameStmt.newname"],
            policy: Quoted,
            render: |n| sql(&Schema::rename(fixed("s"), n_(n))),
        },
        // -- DOMAIN --------------------------------------------------------
        Site {
            id: "ddl/create-domain.name",
            api: "Domain::create(Name, base)",
            kinds: &["CreateDomainStmt.domainname[0]"],
            policy: Quoted,
            render: |n| sql(&Domain::create(n_(n), ColumnType::Integer)),
        },
        Site {
            id: "ddl/create-domain.schema",
            api: "Domain::create((Name, domain), base) — the schema part",
            kinds: &["CreateDomainStmt.domainname[0]"],
            policy: Quoted,
            render: |n| sql(&Domain::create((n_(n), fixed("d")), ColumnType::Integer)),
        },
        Site {
            id: "ddl/create-domain.database",
            api: "Domain::create((Name, schema, domain), base) — the database part",
            kinds: &["CreateDomainStmt.domainname[0]"],
            policy: Quoted,
            render: |n| {
                sql(&Domain::create(
                    (n_(n), fixed("s"), fixed("d")),
                    ColumnType::Integer,
                ))
            },
        },
        Site {
            id: "ddl/create-domain.base",
            api: "Domain::create(name, ColumnType::named(String))",
            kinds: &["CreateDomainStmt.type_name.names[0]"],
            policy: TypePart,
            render: |n| sql(&Domain::create(fixed("d"), ColumnType::named(n))),
        },
        Site {
            id: "ddl/create-domain.collation",
            api: "DomainCreateStatement::collate(Name)",
            kinds: &["CreateDomainStmt.coll_clause.collname[0]"],
            policy: Quoted,
            render: |n| sql(Domain::create(fixed("d"), ColumnType::Text).collate(n_(n))),
        },
        Site {
            id: "ddl/create-domain.check-name",
            api: "DomainCheck::name(Name)",
            kinds: &["Constraint.conname"],
            policy: Quoted,
            render: |n| {
                sql(Domain::create(fixed("d"), ColumnType::Integer)
                    .check(DomainCheck::new(Expr::domain_value().gt(0)).name(n_(n))))
            },
        },
        Site {
            id: "ddl/alter-domain.name",
            api: "Domain::alter(Name)",
            kinds: &["AlterDomainStmt.type_name[0]"],
            policy: Quoted,
            render: |n| sql(&Domain::alter(n_(n)).drop_default()),
        },
        Site {
            id: "ddl/alter-domain.add-check-name",
            api: "PendingDomainAlter::add_check(DomainCheck::name(Name))",
            kinds: &["Constraint.conname"],
            policy: Quoted,
            render: |n| {
                sql(&Domain::alter(fixed("d"))
                    .add_check(DomainCheck::new(Expr::domain_value().gt(0)).name(n_(n))))
            },
        },
        Site {
            id: "ddl/alter-domain.drop-constraint",
            api: "PendingDomainAlter::drop_constraint(Name)",
            kinds: &["AlterDomainStmt.name"],
            policy: Quoted,
            render: |n| sql(&Domain::alter(fixed("d")).drop_constraint(n_(n))),
        },
        Site {
            id: "ddl/alter-domain.validate-constraint",
            api: "PendingDomainAlter::validate_constraint(Name)",
            kinds: &["AlterDomainStmt.name"],
            policy: Quoted,
            render: |n| sql(&Domain::alter(fixed("d")).validate_constraint(n_(n))),
        },
        Site {
            id: "ddl/alter-domain.rename-constraint-from",
            api: "PendingDomainAlter::rename_constraint(Name, to)",
            kinds: &["RenameStmt.subname"],
            policy: Quoted,
            render: |n| sql(&Domain::alter(fixed("d")).rename_constraint(n_(n), fixed("c"))),
        },
        Site {
            id: "ddl/alter-domain.rename-constraint-to",
            api: "PendingDomainAlter::rename_constraint(from, Name)",
            kinds: &["RenameStmt.newname"],
            policy: Quoted,
            render: |n| sql(&Domain::alter(fixed("d")).rename_constraint(fixed("c"), n_(n))),
        },
        Site {
            id: "ddl/alter-domain.rename-name",
            api: "Domain::alter(Name).rename_to(new)",
            kinds: &["RenameStmt.object.List.items[0]"],
            policy: Quoted,
            render: |n| sql(&Domain::alter(n_(n)).rename_to(fixed("e"))),
        },
        Site {
            id: "ddl/alter-domain.rename-to",
            api: "PendingDomainAlter::rename_to(Name)",
            kinds: &["RenameStmt.newname"],
            policy: Quoted,
            render: |n| sql(&Domain::alter(fixed("d")).rename_to(n_(n))),
        },
        Site {
            id: "ddl/drop-domain.name",
            api: "Domain::drop(Name)",
            kinds: &["TypeName.names[0]"],
            policy: Quoted,
            render: |n| sql(&Domain::drop(n_(n))),
        },
        Site {
            id: "ddl/drop-domain.further-name",
            api: "DomainDropStatement::name(Name)",
            kinds: &["TypeName.names[0]"],
            policy: Quoted,
            render: |n| sql(Domain::drop(fixed("d")).name(n_(n))),
        },
        Site {
            id: "ddl/create-table.column-type-domain",
            api: "ColumnType::Domain { name: Name, .. }",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Table::create(fixed("t")).col(ColumnDef::new_with_type(
                    fixed("c"),
                    ColumnType::Domain {
                        name: n_(n),
                        schema: None,
                        base: std::sync::Arc::new(ColumnType::Integer),
                    },
                )))
            },
        },
        Site {
            id: "ddl/create-table.column-type-domain-schema",
            api: "ColumnType::Domain { schema: Some(Name), .. }",
            kinds: &["ColumnDef.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Table::create(fixed("t")).col(ColumnDef::new_with_type(
                    fixed("c"),
                    ColumnType::Domain {
                        name: fixed("d"),
                        schema: Some(n_(n)),
                        base: std::sync::Arc::new(ColumnType::Integer),
                    },
                )))
            },
        },
        // -- EXTENSION -----------------------------------------------------
        Site {
            id: "ddl/create-extension.name",
            api: "Extension::create(Name)",
            kinds: &["CreateExtensionStmt.extname"],
            policy: Quoted,
            render: |n| sql(&Extension::create(n_(n))),
        },
        Site {
            id: "ddl/create-extension.schema",
            api: "ExtensionCreateStatement::schema(Name)",
            kinds: &["DefElem.arg"],
            policy: Quoted,
            render: |n| sql(Extension::create(fixed("e")).schema(n_(n))),
        },
        Site {
            id: "ddl/drop-extension.name",
            api: "Extension::drop(Name)",
            kinds: &["DropStmt.objects[0]"],
            policy: Quoted,
            render: |n| sql(&Extension::drop(n_(n))),
        },
    ]
}

/// The body a view, materialized view or `CREATE TABLE AS` site wraps, which
/// names nothing under test: `SELECT 1`.
fn body() -> SelectStatement {
    Query::select().expr(1).take()
}

/// Why a constructor over [`body`] cannot refuse it: the query has no tie
/// limit and no window for the definition check to refuse.
const BODY_TAKEN: &str = "SELECT 1 is a body every definition takes";
