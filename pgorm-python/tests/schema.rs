//! Native DDL built independently must match Python SQL, including literal escaping.
use pgorm::pgorm_query::{
    Check, ColumnDef, ColumnType, ConstraintChange, ConstraintDrop, Domain, DomainCheck,
    Enforcement, Expr, ExprTrait, Func, GeneratedKind, Index, IndexOrder, IndexType, IntoName,
    MaterializedView, Name, NotNullConstraint, Query, Schema, SelectStatement, SimpleExpr,
    StringLen, Table, TableKey, TableName, TypeName, Values, View, ViewCheckOption, error,
    extension::Type,
};
use pgorm_python::expressions::Compiled;
use pyo3::{prelude::*, types::PyDict};
use std::{collections::BTreeMap, ffi::CString, sync::Arc};

fn a(name: &str) -> Name {
    Name::runtime(name)
}

fn programs() -> error::Result<BTreeMap<&'static str, String>> {
    let table = TableName::SchemaTable(
        a("schema \"雪\"").into_name(),
        a("items \"雪\"").into_name(),
    );
    let kind = (a("schema \"雪\""), a("Mood \"雪\""));
    let column_type = ColumnType::Enum {
        name: kind.1.clone().into_name(),
        schema: Some(kind.0.clone().into_name()),
        variants: vec![],
    };
    let mut base = Table::create(table.clone());
    base.col(ColumnDef::new_with_type(a("id \"x\""), ColumnType::Integer).not_null());
    let full = base
        .clone()
        .col(
            ColumnDef::new_with_type(a("name"), ColumnType::String(StringLen::N(80)))
                .default("O'Brien \\ 雪"),
        )
        .col(
            ColumnDef::new_with_type(a("mood"), column_type.clone()).default(
                Expr::val("busy")
                    .cast_as_type(TypeName::new(kind.1.clone()).schema(kind.0.clone())),
            ),
        )
        .col(ColumnDef::new_with_type(
            a("moods"),
            ColumnType::Array(Arc::new(column_type)),
        ))
        .col(ColumnDef::new_with_type(
            a("amount"),
            ColumnType::Decimal(Some((12, 3))),
        ))
        .primary_key(a("id \"x\""))
        .unique(
            TableKey::new(a("name"))
                .nulls_not_distinct()
                .name(a("unique \"x\"")),
        )
        .check(Expr::col(a("id \"x\"")).gt(0i64))
        .if_not_exists()
        .to_string();
    let programs = [
        ("table", full),
        (
            "generated",
            base.clone()
                .col(
                    ColumnDef::new_with_type(a("twice"), ColumnType::Integer)
                        .generated(Expr::col(a("id \"x\"")).mul(2i64), GeneratedKind::Stored),
                )
                .to_string(),
        ),
        (
            "generated_virtual",
            base.col(
                ColumnDef::new_with_type(a("next"), ColumnType::Integer)
                    .generated(Expr::col(a("id \"x\"")).add(1i64), GeneratedKind::Virtual),
            )
            .to_string(),
        ),
        (
            "column_specs",
            Table::create(table.clone())
                .col(ColumnDef::new_with_type(a("id"), ColumnType::BigInteger).auto_increment())
                .col(
                    ColumnDef::new_with_type(a("n"), ColumnType::Integer)
                        .null()
                        .check(Expr::col(a("n")).gt(0i64)),
                )
                .primary_key(a("id"))
                .unique(a("n"))
                .to_string(),
        ),
        (
            "index",
            Index::create(table.clone(), a("name"))
                .name(a("index \"x\""))
                .col((a("id \"x\""), IndexOrder::Desc))
                .unique()
                .nulls_not_distinct()
                .index_type(IndexType::BTree)
                .if_not_exists()
                .to_string(),
        ),
        (
            "index_gin",
            Index::create(table.clone(), a("moods"))
                .index_type(IndexType::Gin)
                .to_string(),
        ),
        (
            "drop_index",
            Index::drop(a("index \"x\""))
                .table(table.clone())
                .if_exists()
                .to_string(),
        ),
        (
            "drop_table",
            Table::drop(table.clone()).if_exists().cascade().to_string(),
        ),
        (
            "rename_table",
            Table::rename(table.clone(), a("new \"x\"")).to_string(),
        ),
        (
            "rename_column",
            Table::rename_column(table.clone(), a("id \"x\""), a("new \"x\"")).to_string(),
        ),
        ("truncate_table", Table::truncate(table.clone()).to_string()),
        (
            "add_column",
            Table::alter(table.clone())
                .add_column_if_not_exists(ColumnDef::new_with_type(a("extra"), ColumnType::Text))
                .to_string(),
        ),
        (
            "modify_column",
            Table::alter(table.clone())
                .modify_column(
                    ColumnDef::new_with_type(a("extra"), ColumnType::String(StringLen::None))
                        .not_null()
                        .default("hello"),
                )
                .to_string(),
        ),
        (
            "collated",
            Table::create(table.clone())
                .col(
                    ColumnDef::new_with_type(a("word \"x\""), ColumnType::Text)
                        .not_null()
                        .collate(a("C")),
                )
                .col(
                    ColumnDef::new_with_type(a("label"), ColumnType::Text)
                        .collate((a("pg_catalog"), a("POSIX"))),
                )
                .to_string(),
        ),
        (
            "add_column_collated",
            Table::alter(table.clone())
                .add_column(
                    ColumnDef::new_with_type(a("extra"), ColumnType::Text)
                        .collate((a("schema \"雪\""), a("C \"x\""))),
                )
                .to_string(),
        ),
        (
            "modify_column_collated",
            Table::alter(table.clone())
                .modify_column(
                    ColumnDef::new_with_type(a("extra"), ColumnType::String(StringLen::None))
                        .collate(a("C")),
                )
                .to_string(),
        ),
        (
            "add_primary_key",
            Table::alter(table.clone())
                .add_primary_key((a("id \"x\""), a("name")))
                .to_string(),
        ),
        (
            "add_unique",
            Table::alter(table.clone())
                .add_unique(
                    TableKey::new(a("name"))
                        .col(a("id \"x\""))
                        .name(a("unique \"x\""))
                        .nulls_not_distinct(),
                )
                .to_string(),
        ),
        (
            "temporal_keys",
            Table::create(table.clone())
                .col(ColumnDef::new_with_type(a("id \"x\""), ColumnType::Integer).not_null())
                .col(ColumnDef::new_with_type(
                    a("during \"x\""),
                    ColumnType::Enum {
                        name: a("tstzrange").into_name(),
                        schema: None,
                        variants: vec![],
                    },
                ))
                .primary_key(TableKey::new(a("id \"x\"")).without_overlaps(a("during \"x\"")))
                .unique(
                    TableKey::new(a("name"))
                        .col(a("id \"x\""))
                        .name(a("unique \"x\""))
                        .nulls_not_distinct()
                        .without_overlaps(a("during \"x\"")),
                )
                .to_string(),
        ),
        (
            "add_primary_key_temporal",
            Table::alter(table.clone())
                .add_primary_key(TableKey::new(a("id \"x\"")).without_overlaps(a("during \"x\"")))
                .to_string(),
        ),
        (
            "add_unique_temporal",
            Table::alter(table.clone())
                .add_unique(TableKey::new(a("name")).without_overlaps(a("during \"x\"")))
                .to_string(),
        ),
        (
            "drop_column",
            Table::alter(table.clone())
                .drop_column(a("extra"))
                .to_string(),
        ),
        (
            "set_expression",
            Table::alter(table.clone())
                .set_expression(a("twice"), Expr::col(a("id \"x\"")).mul(3i64))
                .to_string(),
        ),
        (
            "drop_expression",
            Table::alter(table.clone())
                .drop_expression(a("twice"))
                .to_string(),
        ),
        (
            "drop_expression_if_exists",
            Table::alter(table.clone())
                .drop_expression_if_exists(a("twice"))
                .to_string(),
        ),
        (
            "not_null_named",
            Table::create(table.clone())
                .col(
                    ColumnDef::new_with_type(a("extra"), ColumnType::Text)
                        .not_null_named(a("present \"x\""))
                        .not_null_no_inherit(),
                )
                .col(ColumnDef::new_with_type(a("kept"), ColumnType::Text).not_null_no_inherit())
                .to_string(),
        ),
        (
            "add_not_null",
            Table::alter(table.clone())
                .add_not_null(NotNullConstraint::new(a("extra")))
                .to_string(),
        ),
        (
            "add_not_null_named",
            Table::alter(table.clone())
                .add_not_null(
                    NotNullConstraint::new(a("extra"))
                        .name(a("present \"x\""))
                        .no_inherit()
                        .not_valid(),
                )
                .to_string(),
        ),
        (
            "validate_constraint",
            Table::alter(table.clone())
                .validate_constraint(a("present \"x\""))
                .to_string(),
        ),
        (
            "alter_constraint_inherit",
            Table::alter(table.clone())
                .alter_constraint(a("present \"x\""), ConstraintChange::Inherit)
                .to_string(),
        ),
        (
            "alter_constraint_no_inherit",
            Table::alter(table.clone())
                .alter_constraint(a("present \"x\""), ConstraintChange::NoInherit)
                .to_string(),
        ),
        (
            "drop_constraint",
            Table::alter(table.clone())
                .drop_constraint(a("present \"x\""))
                .to_string(),
        ),
        (
            "drop_constraint_if_exists",
            Table::alter(table.clone())
                .drop_constraint(
                    ConstraintDrop::new(a("present \"x\""))
                        .if_exists()
                        .cascade(),
                )
                .to_string(),
        ),
        (
            "rename_constraint",
            Table::rename_constraint(table.clone(), a("present \"x\""), a("kept \"x\""))
                .to_string(),
        ),
        (
            "check_named",
            Table::create(table.clone())
                .col(
                    ColumnDef::new_with_type(a("n"), ColumnType::Integer).check(
                        Check::new(Expr::col(a("n")).gt(0i64))
                            .name(a("positive \"x\""))
                            .enforcement(Enforcement::NotEnforced),
                    ),
                )
                .check(Check::new(Expr::col(a("n")).lt(100i64)).name(a("small \"x\"")))
                .check(Check::new(Expr::col(a("n")).ne(7i64)).enforcement(Enforcement::NotEnforced))
                .to_string(),
        ),
        (
            "add_check",
            Table::alter(table.clone())
                .add_check(
                    Check::new(Expr::col(a("n")).gt(0i64))
                        .name(a("positive \"x\""))
                        .enforcement(Enforcement::NotEnforced),
                )
                .to_string(),
        ),
        (
            "add_check_plain",
            Table::alter(table.clone())
                .add_check(Expr::col(a("n")).gt(0i64))
                .to_string(),
        ),
        (
            "add_check_not_valid",
            Table::alter(table.clone())
                .add_check(
                    Check::new(Expr::col(a("n")).gt(0i64))
                        .name(a("positive \"x\""))
                        .no_inherit()
                        .not_valid(),
                )
                .to_string(),
        ),
        (
            "check_no_inherit",
            Table::create(table.clone())
                .col(
                    ColumnDef::new_with_type(a("n"), ColumnType::Integer)
                        .check(Check::new(Expr::col(a("n")).gt(0i64)).no_inherit()),
                )
                .check(
                    Check::new(Expr::col(a("n")).lt(100i64))
                        .name(a("small \"x\""))
                        .no_inherit()
                        .enforcement(Enforcement::NotEnforced),
                )
                .to_string(),
        ),
        (
            "alter_constraint_enforced",
            Table::alter(table.clone())
                .alter_constraint(a("fk \"x\""), ConstraintChange::Enforced)
                .to_string(),
        ),
        (
            "alter_constraint_not_enforced",
            Table::alter(table.clone())
                .alter_constraint(a("fk \"x\""), ConstraintChange::NotEnforced)
                .to_string(),
        ),
        (
            "enum",
            Type::create(kind.clone())
                .as_enum()
                .values(["", "O'Brien \\ 雪", "busy"])
                .to_string(),
        ),
        (
            "enum_before",
            Type::alter(kind.clone())
                .add_value("new")
                .before("busy")
                .to_string(),
        ),
        (
            "enum_after",
            Type::alter(kind.clone())
                .add_value("new")
                .after("busy")
                .to_string(),
        ),
        (
            "enum_rename_value",
            Type::alter(kind.clone())
                .rename_value("busy", "calm")
                .to_string(),
        ),
        (
            "enum_rename",
            Type::alter(kind.clone())
                .rename_to(a("new \"x\""))
                .to_string(),
        ),
        (
            "enum_drop",
            Type::drop(kind).if_exists().cascade().to_string(),
        ),
    ]
    .into_iter()
    .chain(views(&table)?)
    .chain(schemas())
    .chain(domains())
    .collect();
    Ok(programs)
}

/// The statements over a schema, its name carrying a quote and a non-ASCII
/// character.
fn schemas() -> BTreeMap<&'static str, String> {
    let name = || a("schema \"雪\"");
    BTreeMap::from([
        ("create_schema", Schema::create(name()).to_string()),
        (
            "create_schema_if_not_exists",
            Schema::create(name()).if_not_exists().to_string(),
        ),
        ("drop_schema", Schema::drop(name()).to_string()),
        (
            "drop_schemas",
            Schema::drop(name())
                .name(a("archive"))
                .name(a("Old \"x\""))
                .if_exists()
                .cascade()
                .to_string(),
        ),
        (
            "rename_schema",
            Schema::rename(name(), a("renamed \"雪\"")).to_string(),
        ),
    ])
}

/// The statements over a domain, its name schema-qualified and carrying a
/// quote and a non-ASCII character.
fn domains() -> BTreeMap<&'static str, String> {
    let name = || (a("schema \"雪\""), a("Qty \"雪\""));
    let value = Expr::domain_value;
    let small = || a("small \"x\"");
    let column_type = ColumnType::Enum {
        name: a("Qty \"雪\""),
        schema: Some(a("schema \"雪\"")),
        variants: vec![],
    };
    BTreeMap::from([
        (
            "create_domain",
            Domain::create(a("qty \"雪\""), ColumnType::Integer).to_string(),
        ),
        (
            "create_domain_clauses",
            Domain::create(name(), ColumnType::Text)
                .check(DomainCheck::new(value().ne("")).name(a("filled \"x\"")))
                .not_null()
                .default("O'Brien \\ 雪")
                .collate((a("pg_catalog"), a("C")))
                .check(value().ne("none"))
                .to_string(),
        ),
        (
            "create_domain_over_array",
            Domain::create(
                name(),
                ColumnType::Array(Arc::new(ColumnType::String(StringLen::N(20)))),
            )
            .null()
            .default(Expr::expr(SimpleExpr::Constant(1i64.into())).add(2i64))
            .to_string(),
        ),
        (
            "create_domain_over_domain",
            Domain::create(a("small"), column_type)
                .not_null()
                .null()
                .to_string(),
        ),
        (
            "set_domain_default",
            Domain::alter(name()).set_default(5i64).to_string(),
        ),
        (
            "drop_domain_default",
            Domain::alter(name()).drop_default().to_string(),
        ),
        (
            "set_domain_not_null",
            Domain::alter(name()).set_not_null().to_string(),
        ),
        (
            "drop_domain_not_null",
            Domain::alter(name()).drop_not_null().to_string(),
        ),
        (
            "add_domain_check",
            Domain::alter(name())
                .add_check(value().gt(0i64))
                .to_string(),
        ),
        (
            "add_domain_check_not_valid",
            Domain::alter(name())
                .add_check(
                    DomainCheck::new(value().lt(1000i64))
                        .name(small())
                        .not_valid(),
                )
                .to_string(),
        ),
        (
            "drop_domain_constraint",
            Domain::alter(name())
                .drop_constraint(ConstraintDrop::new(small()).if_exists().cascade())
                .to_string(),
        ),
        (
            "rename_domain_constraint",
            Domain::alter(name())
                .rename_constraint(small(), a("tiny \"雪\""))
                .to_string(),
        ),
        (
            "validate_domain_constraint",
            Domain::alter(name())
                .validate_constraint(small())
                .to_string(),
        ),
        (
            "rename_domain",
            Domain::alter(name())
                .rename_to(a("units \"雪\""))
                .to_string(),
        ),
        ("drop_domain", Domain::drop(name()).to_string()),
        (
            "drop_domains",
            Domain::drop(name())
                .name(a("plain"))
                .name((a("archive"), a("other")))
                .if_exists()
                .cascade()
                .to_string(),
        ),
    ])
}

/// The view, materialized view and `CREATE TABLE AS` statements over one
/// query, whose string value is written into each definition as a literal.
fn views(table: &TableName) -> error::Result<BTreeMap<&'static str, String>> {
    let query = || -> SelectStatement {
        Query::select()
            .column(a("id \"x\""))
            .column(a("name"))
            .from(table.clone())
            .and_where(Expr::col(a("name")).ne("O'Brien \\ 雪"))
            .take()
    };
    let view = TableName::SchemaTable(a("schema \"雪\"").into_name(), a("view \"雪\"").into_name());
    Ok(BTreeMap::from([
        (
            "create_view",
            View::create(view.clone(), query())?.to_string(),
        ),
        (
            "create_view_options",
            View::create(view.clone(), query())?
                .columns([a("key \"x\""), a("label")])
                .or_replace()
                .check_option(ViewCheckOption::Local)
                .security_barrier()
                .security_invoker()
                .to_string(),
        ),
        (
            "create_view_cascaded",
            View::create(view.clone(), query())?
                .check_option(ViewCheckOption::Cascaded)
                .to_string(),
        ),
        (
            "create_recursive_view",
            View::create_recursive(view.clone(), a("key \"x\""), query())?
                .column(a("label"))
                .or_replace()
                .security_barrier()
                .to_string(),
        ),
        (
            "drop_view",
            View::drop(view.clone()).if_exists().cascade().to_string(),
        ),
        (
            "create_materialized_view",
            MaterializedView::create(view.clone(), query())?
                .columns([a("key \"x\"")])
                .if_not_exists()
                .with_no_data()
                .to_string(),
        ),
        (
            "refresh_materialized_view",
            MaterializedView::refresh(view.clone()).to_string(),
        ),
        (
            "refresh_concurrently",
            MaterializedView::refresh(view.clone())
                .concurrently()
                .to_string(),
        ),
        (
            "refresh_with_no_data",
            MaterializedView::refresh(view.clone())
                .with_no_data()
                .to_string(),
        ),
        (
            "drop_materialized_view",
            MaterializedView::drop(view.clone()).if_exists().to_string(),
        ),
        (
            "create_table_as",
            Table::create_as(view, query())?
                .columns([a("key \"x\""), a("label")])
                .if_not_exists()
                .with_no_data()
                .to_string(),
        ),
    ]))
}

// [spec:pgorm:req:bindings.schema-tables/test]
// [spec:pgorm:req:bindings.collations/test]    a column's collation, qualified or bare and a
// later call replacing an earlier, renders as the Rust builder renders it in each position
// [spec:pgorm:req:bindings.schema-views+2/test]    every view, materialized view and CREATE TABLE AS
// option renders as the Rust builder renders it
// [spec:pgorm:req:bindings.schema-schemas/test]    every schema create, drop and rename renders as
// the Rust builder renders it
// [spec:pgorm:req:bindings.schema-domains+2/test]    every domain create, alter action and drop renders
// as the Rust builder renders it
#[test]
fn ddl_matches_native_rust() -> Result<(), Box<dyn std::error::Error>> {
    Python::initialize();
    Python::attach(|py| -> Result<(), Box<dyn std::error::Error>> {
        let native = PyModule::new(py, "pgorm._native")?;
        pgorm_python::install(&native, Default::default())?;
        let source = CString::new(include_str!("schema_programs.py"))?;
        let examples = PyModule::from_code(py, &source, c"schema_programs.py", c"schema_programs")?;
        let queries = examples.call_method1("programs", (&native,))?;
        let rust = programs()?;
        assert_eq!(queries.len()?, rust.len());
        for (name, sql) in rust {
            let query = queries.get_item(name)?;
            let compiled: Compiled = query
                .call_method0("inspect")?
                .extract()
                .map_err(PyErr::from)?;
            assert_eq!(compiled.sql, sql, "{name}");
            assert_eq!(compiled.values, Values(vec![]), "{name}");
            let executable = pgorm_python::statements::compile(&query)?;
            assert_eq!(
                (executable.sql, executable.values),
                (compiled.sql, compiled.values)
            );
        }
        Ok(())
    })
}

// [spec:pgorm:req:bindings.schema-views+2/test]    each view builder refuses an unordered tie limit
// and an undeclared window with the error Rust's own checks give
#[test]
fn view_bodies_refused_as_rust_refuses() -> Result<(), Box<dyn std::error::Error>> {
    let mut unordered = Query::select();
    unordered
        .column(a("id"))
        .from(a("account"))
        .limit_with_ties(2);
    let mut undeclared = Query::select();
    undeclared
        .expr_window_name(Func::count(Expr::col(a("id"))), a("w"))
        .from(a("account"));
    let refusals = [
        (
            "unordered",
            unordered.check_tie_limit().map_err(|e| e.to_string()),
        ),
        (
            "undeclared",
            undeclared.check_windows().map_err(|e| e.to_string()),
        ),
    ];
    Python::initialize();
    Python::attach(|py| -> Result<(), Box<dyn std::error::Error>> {
        let native = PyModule::new(py, "pgorm._native")?;
        pgorm_python::install(&native, Default::default())?;
        let globals = PyDict::new(py);
        globals.set_item("p", &native)?;
        py.run(
            c"account = p.Table('account')
unordered = p.Select(p.col('id')).from_(account).limit_with_ties(2)
undeclared = p.Select(p.call('count', p.col('id')).over('w')).from_(account)
step = p.Select(p.col('n')).from_(p.Table('r'))
builders = {
    'create_view': lambda q: p.create_view(p.Table('v'), q),
    'create_recursive_view': lambda q: p.create_recursive_view(p.Table('r'), ['n'], q.union_all(step)),
    'create_materialized_view': lambda q: p.create_materialized_view(p.Table('m'), q),
    'create_table_as': lambda q: p.create_table_as(p.Table('t'), q),
}",
            Some(&globals),
            None,
        )?;
        let refused = native.getattr("ConstructionError")?;
        let builders = globals
            .get_item("builders")?
            .ok_or("the program defines its builders")?;
        let builders = builders.cast::<PyDict>().map_err(PyErr::from)?;
        assert_eq!(builders.len(), 4);
        for (name, build) in builders.iter() {
            for (query, expected) in &refusals {
                let Err(expected) = expected else {
                    panic!("Rust admits the {query} query");
                };
                let query = globals.get_item(query)?.ok_or("the program defines it")?;
                let error = build.call1((query,)).expect_err("the body is refused");
                assert!(error.is_instance(py, &refused), "{name}: {error}");
                assert_eq!(&error.value(py).to_string(), expected, "{name}");
            }
        }
        Ok(())
    })
}
