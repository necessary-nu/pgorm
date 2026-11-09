//! The registry: every public API that renders a caller-supplied name into
//! SQL text, one entry per API, each with a render closure that builds the
//! smallest statement putting the name in that position.
//!
//! # Adding a site
//!
//! A new public API that writes a caller-supplied name into SQL is incomplete
//! until it has an entry here (the `security.ident-oracle` rule,
//! `docs/spec/ident-oracle.md`).
//! Append a [`Site`] to the section for its crate and area:
//!
//! - `id` — `<crate>/<statement>.<position>`, unique;
//! - `api` — the public entry point, as a caller spells it;
//! - `kinds` — where the name must land in libpg_query's parse tree, one entry
//!   per occurrence: the innermost node holding it and the field path to the
//!   value (`RangeVar.relname`, `ColumnRef.fields[1]`, `ResTarget.name`). A
//!   wrong guess fails with the positions the benign rendering actually
//!   produced, so the entry can be checked against them rather than guessed;
//! - `policy` — how the site writes the name ([`Policy`]);
//! - `render` — build the statement with the name `n` in the position and
//!   nothing else caller-supplied, and return its text (or the API's refusal).
//!
//! Every other identifier in the statement is fixed (`t`, `c`, `u`, …), so the
//! only thing the oracle's tree comparison can find moving is the name.
//! The registry is split by area, one file each, all listed by [`sites`]:
//! this file holds pgorm-query's DML and expression sites, `registry_ddl.rs`
//! its DDL sites, `registry_orm.rs` pgorm's, `registry_disjoint.rs` the
//! disjoint schema's and the one-of `CHECK`'s, and `live_capture.rs` the sites
//! whose statements exist only against a live server (pgorm-pool's savepoint,
//! the migration ledger, the cursor's qualifiers).

use pgorm::pgorm_query::{
    Asterisk, ColumnType, CommonTableExpression, Cycle, Expr, ExtendedWindow, FromItem, Func,
    IntoNamedTable, JoinType, JsonTable, JsonTableColumn, LockType, MatchedAction, MergeInsert,
    MergeUpdate, Name, OnConflict, Order, Query, Range, RecursiveWithClause, ReturningRow,
    SampleMethod, Search, SearchOrder, SqlTemplate, TableFunction, TableSample, TypeName,
    WindowStatement, WithClause,
};
use pgorm_query::ExprTrait;

use super::oracle::{
    Policy::{FunctionName, Quoted, TypePart},
    Rendered, Site,
};

/// The name under test.
pub fn n_(name: &str) -> Name {
    Name::runtime(name)
}

/// A fixed identifier the statement needs besides the name under test.
pub fn fixed(name: &'static str) -> Name {
    Name::runtime(name)
}

/// A statement's inline (`to_string`) rendering. Names render identically on
/// the `build()` path; only values differ, and no site here binds one that
/// the name could reach.
pub fn sql<S: std::fmt::Display + ?Sized>(statement: &S) -> Rendered {
    Rendered::sql(statement.to_string())
}

/// Every registered site, pgorm-query's then pgorm's.
pub fn sites() -> Vec<Site> {
    let mut sites = query_sites();
    sites.extend(super::registry_ddl::sites());
    sites.extend(super::registry_orm::sites());
    sites.extend(super::registry_disjoint::sites());
    sites
}

// ---------------------------------------------------------------------------
// pgorm-query: SELECT, INSERT, UPDATE, DELETE, MERGE and their expressions
// ---------------------------------------------------------------------------

fn query_sites() -> Vec<Site> {
    vec![
        // -- projection and column references ------------------------------
        Site {
            id: "query/select.column",
            api: "SelectStatement::column(Name)",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| sql(Query::select().column(n_(n)).from(fixed("t"))),
        },
        Site {
            id: "query/select.column.table-qualifier",
            api: "SelectStatement::column((Name, Name)) — the table part",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| sql(Query::select().column((n_(n), fixed("c"))).from(fixed("t"))),
        },
        Site {
            id: "query/select.column.qualified",
            api: "SelectStatement::column((Name, Name)) — the column part",
            kinds: &["ColumnRef.fields[1]"],
            policy: Quoted,
            render: |n| sql(Query::select().column((fixed("t"), n_(n))).from(fixed("t"))),
        },
        Site {
            id: "query/select.column.schema-qualifier",
            api: "SelectStatement::column((Name, Name, Name)) — the schema part",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column((n_(n), fixed("t"), fixed("c")))
                    .from(fixed("t")))
            },
        },
        Site {
            id: "query/select.column.table-asterisk",
            api: "SelectStatement::column((Name, Asterisk))",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| sql(Query::select().column((n_(n), Asterisk)).from(fixed("t"))),
        },
        Site {
            id: "query/select.expr-as.alias",
            api: "SelectStatement::expr_as(expr, Name)",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| sql(Query::select().expr_as(Expr::val(1), n_(n))),
        },
        Site {
            id: "query/select.distinct-on",
            api: "SelectStatement::distinct_on([Name])",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .distinct_on([n_(n)])
                    .column(fixed("c"))
                    .from(fixed("t")))
            },
        },
        Site {
            id: "query/where.expr-col",
            api: "Expr::col(Name) in a WHERE predicate",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(fixed("c"))
                    .from(fixed("t"))
                    .and_where(Expr::col(n_(n)).eq(1)))
            },
        },
        // -- FROM items ----------------------------------------------------
        Site {
            id: "query/select.from.table",
            api: "SelectStatement::from(Name)",
            kinds: &["RangeVar.relname"],
            policy: Quoted,
            render: |n| sql(Query::select().column(Asterisk).from(n_(n))),
        },
        Site {
            id: "query/select.from.schema",
            api: "SelectStatement::from((Name, Name)) — the schema part",
            kinds: &["RangeVar.schemaname"],
            policy: Quoted,
            render: |n| sql(Query::select().column(Asterisk).from((n_(n), fixed("t")))),
        },
        Site {
            id: "query/select.from-as.alias",
            api: "SelectStatement::from_as(table, Name)",
            kinds: &["RangeVar.alias.aliasname"],
            policy: Quoted,
            render: |n| sql(Query::select().column(Asterisk).from_as(fixed("t"), n_(n))),
        },
        Site {
            id: "query/select.named-table.alias",
            api: "NamedTable::alias(Name) through SelectStatement::from",
            kinds: &["RangeVar.alias.aliasname"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(Asterisk)
                    .from(fixed("t").into_named_table().alias(n_(n))))
            },
        },
        Site {
            id: "query/select.from-subquery.alias",
            api: "SelectStatement::from_subquery(query, Name)",
            kinds: &["RangeSubselect.alias.aliasname"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(Asterisk)
                    .from_subquery(Query::select().expr(Expr::val(1)).take(), n_(n)))
            },
        },
        Site {
            id: "query/select.from-values.alias",
            api: "SelectStatement::from_values(rows, Name)",
            kinds: &["RangeSubselect.alias.aliasname"],
            policy: Quoted,
            render: |n| sql(Query::select().column(Asterisk).from_values([1], n_(n))),
        },
        Site {
            id: "query/select.from-function.alias",
            api: "SelectStatement::from_function(call, Name)",
            kinds: &["RangeFunction.alias.aliasname"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(Asterisk)
                    .from_function(Func::named(fixed("f")).arg(1), n_(n)))
            },
        },
        Site {
            id: "query/select.from-function.name",
            api: "SelectStatement::from_function(Func::named(Name), alias)",
            kinds: &["FuncCall.funcname[0]"],
            policy: FunctionName,
            render: |n| {
                sql(Query::select()
                    .column(Asterisk)
                    .from_function(Func::named(n_(n)).arg(1).arg(2), fixed("f")))
            },
        },
        Site {
            id: "query/select.from-template.alias",
            api: "FromItem::Template(SqlTemplate, Name)",
            kinds: &["RangeSubselect.alias.aliasname"],
            policy: Quoted,
            render: |n| match SqlTemplate::from_sql("SELECT 1", []) {
                Ok(template) => sql(Query::select()
                    .column(Asterisk)
                    .from(FromItem::Template(template, n_(n)))),
                Err(err) => Rendered::Refused(format!("{err:?}")),
            },
        },
        Site {
            id: "query/select.table-function.alias",
            api: "TableFunction::alias(Name)",
            kinds: &["RangeFunction.alias.aliasname"],
            policy: Quoted,
            render: |n| {
                sql(Query::select().column(Asterisk).from(
                    TableFunction::new(Func::named(fixed("f")).arg(1))
                        .with_ordinality()
                        .alias(n_(n)),
                ))
            },
        },
        Site {
            id: "query/select.table-function.column",
            api: "TableFunction::columns([Name])",
            kinds: &["RangeFunction.alias.colnames[0]"],
            policy: Quoted,
            render: |n| {
                sql(Query::select().column(Asterisk).from(
                    TableFunction::new(Func::named(fixed("f")).arg(1))
                        .columns([n_(n)])
                        .alias(fixed("t")),
                ))
            },
        },
        Site {
            id: "query/select.table-sample.table",
            api: "TableSample::new(Name, method, percentage)",
            kinds: &["RangeVar.relname"],
            policy: Quoted,
            render: |n| {
                sql(Query::select().column(Asterisk).from(TableSample::new(
                    n_(n),
                    SampleMethod::Bernoulli,
                    10,
                )))
            },
        },
        Site {
            id: "query/select.table-sample.alias",
            api: "TableSample::new(NamedTable::alias(Name), method, percentage)",
            kinds: &["RangeVar.alias.aliasname"],
            policy: Quoted,
            render: |n| {
                sql(Query::select().column(Asterisk).from(TableSample::new(
                    fixed("t").into_named_table().alias(n_(n)),
                    SampleMethod::System,
                    10,
                )))
            },
        },
        // -- joins ---------------------------------------------------------
        Site {
            id: "query/select.join.table",
            api: "SelectStatement::left_join(Name, condition)",
            kinds: &["RangeVar.relname"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(Asterisk)
                    .from(fixed("t"))
                    .left_join(n_(n), Expr::val(true).eq(true)))
            },
        },
        Site {
            id: "query/select.join-as.alias",
            api: "SelectStatement::join_as(type, table, Name, condition)",
            kinds: &["RangeVar.alias.aliasname"],
            policy: Quoted,
            render: |n| {
                sql(Query::select().column(Asterisk).from(fixed("t")).join_as(
                    JoinType::InnerJoin,
                    fixed("u"),
                    n_(n),
                    Expr::val(true).eq(true),
                ))
            },
        },
        Site {
            id: "query/select.join-subquery.alias",
            api: "SelectStatement::join_subquery(type, query, Name, condition)",
            kinds: &["RangeSubselect.alias.aliasname"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(Asterisk)
                    .from(fixed("t"))
                    .join_subquery(
                        JoinType::InnerJoin,
                        Query::select().expr(Expr::val(1)).take(),
                        n_(n),
                        Expr::val(true).eq(true),
                    ))
            },
        },
        Site {
            id: "query/select.join-lateral.alias",
            api: "SelectStatement::join_lateral(type, query, Name, condition)",
            kinds: &["RangeSubselect.alias.aliasname"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(Asterisk)
                    .from(fixed("t"))
                    .join_lateral(
                        JoinType::InnerJoin,
                        Query::select().expr(Expr::val(1)).take(),
                        n_(n),
                        Expr::val(true).eq(true),
                    ))
            },
        },
        // -- grouping, ordering, windows, locking --------------------------
        Site {
            id: "query/select.group-by",
            api: "SelectStatement::group_by_col(Name)",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(fixed("c"))
                    .from(fixed("t"))
                    .group_by_col(n_(n)))
            },
        },
        Site {
            id: "query/select.order-by",
            api: "OrderedStatement::order_by(Name, order)",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(fixed("c"))
                    .from(fixed("t"))
                    .order_by(n_(n), Order::Desc))
            },
        },
        Site {
            id: "query/select.window.name",
            api: "SelectStatement::window(Name, window)",
            kinds: &["WindowDef.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(fixed("c"))
                    .from(fixed("t"))
                    .window(n_(n), WindowStatement::partition_by(fixed("c"))))
            },
        },
        Site {
            id: "query/select.window.extends",
            api: "ExtendedWindow::new(Name)",
            kinds: &["WindowDef.refname"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(fixed("c"))
                    .from(fixed("t"))
                    .window(fixed("w"), ExtendedWindow::new(n_(n))))
            },
        },
        Site {
            id: "query/select.over.window-name",
            api: "SelectStatement::expr_window_name(call, Name)",
            kinds: &["FuncCall.over.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .expr_window_name(Func::count(Expr::col(fixed("c"))), n_(n))
                    .from(fixed("t")))
            },
        },
        Site {
            id: "query/select.over.alias",
            api: "SelectStatement::expr_window_as(call, window, Name)",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .expr_window_as(
                        Func::count(Expr::col(fixed("c"))),
                        WindowStatement::partition_by(fixed("c")),
                        n_(n),
                    )
                    .from(fixed("t")))
            },
        },
        Site {
            id: "query/select.window.partition-by",
            api: "WindowStatement::partition_by(Name)",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .expr_window(
                        Func::count(Expr::col(fixed("c"))),
                        WindowStatement::partition_by(n_(n)),
                    )
                    .from(fixed("t")))
            },
        },
        Site {
            id: "query/select.lock.of",
            api: "SelectStatement::lock_with_tables(type, [Name])",
            kinds: &["RangeVar.relname"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(Asterisk)
                    .from(fixed("t"))
                    .lock_with_tables(LockType::Update, [n_(n)]))
            },
        },
        // -- functions and casts -------------------------------------------
        Site {
            id: "query/expr.func.named",
            api: "Func::named(Name)",
            kinds: &["FuncCall.funcname[0]"],
            policy: FunctionName,
            render: |n| sql(Query::select().expr(Func::named(n_(n)).arg(1).arg(2))),
        },
        Site {
            id: "query/expr.cast.type",
            api: "Expr::cast_as(Name)",
            kinds: &["TypeCast.type_name.names[0]"],
            policy: TypePart,
            render: |n| sql(Query::select().expr(Expr::val(1).cast_as(n_(n)))),
        },
        Site {
            id: "query/expr.cast.schema",
            api: "Expr::cast_as_type(TypeName::new(type).schema(Name))",
            kinds: &["TypeCast.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Query::select()
                    .expr(Expr::val(1).cast_as_type(TypeName::new(fixed("ty")).schema(n_(n)))))
            },
        },
        Site {
            id: "query/expr.cast.qualified-type",
            api: "Expr::cast_as_type(TypeName::new(Name).schema(schema))",
            kinds: &["TypeCast.type_name.names[1]"],
            policy: TypePart,
            render: |n| {
                sql(Query::select()
                    .expr(Expr::val(1).cast_as_type(TypeName::new(n_(n)).schema(fixed("s")))))
            },
        },
        // A `TypeName::raw` type expression is program text and renders as
        // written; a schema added to one is a name like any other.
        Site {
            id: "query/expr.cast-raw.schema",
            api: "Expr::cast_as_type(TypeName::raw(type).schema(Name))",
            kinds: &["TypeCast.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Query::select()
                    .expr(Expr::val(1).cast_as_type(TypeName::raw("ty").schema(n_(n)))))
            },
        },
        Site {
            id: "query/expr.cast.array",
            api: "Expr::cast_as_type(TypeName::new(Name).array())",
            kinds: &["TypeCast.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Query::select().expr(Expr::val(1).cast_as_type(TypeName::new(n_(n)).array())))
            },
        },
        Site {
            id: "query/expr.as-range.type",
            api: "Expr::as_range(TypeName::new(Name))",
            kinds: &["TypeCast.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Query::select()
                    .expr(Expr::val(Range::from(1..5)).as_range(TypeName::new(n_(n)))))
            },
        },
        Site {
            id: "query/expr.as-range.schema",
            api: "Expr::as_range(TypeName::new(type).schema(Name))",
            kinds: &["TypeCast.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                sql(Query::select().expr(
                    Expr::val(Range::from(1..5)).as_range(TypeName::new(fixed("ty")).schema(n_(n))),
                ))
            },
        },
        // -- collations -----------------------------------------------------
        Site {
            id: "query/expr.collate.name",
            api: "Expr::collate(Name)",
            kinds: &["CollateClause.collname[0]"],
            policy: Quoted,
            render: |n| sql(Query::select().expr(Expr::col(fixed("c")).collate(n_(n)))),
        },
        Site {
            id: "query/expr.collate.schema",
            api: "Expr::collate((Name, Name)) — the schema part",
            kinds: &["CollateClause.collname[0]"],
            policy: Quoted,
            render: |n| {
                sql(Query::select().expr(Expr::col(fixed("c")).collate((n_(n), fixed("co")))))
            },
        },
        Site {
            id: "query/expr.collate.qualified",
            api: "Expr::collate((Name, Name)) — the collation part",
            kinds: &["CollateClause.collname[1]"],
            policy: Quoted,
            render: |n| {
                sql(Query::select().expr(Expr::col(fixed("c")).collate((fixed("s"), n_(n)))))
            },
        },
        Site {
            id: "query/select.order-by.collate",
            api: "OrderedStatement::order_by_expr(Expr::collate(Name), order)",
            kinds: &["CollateClause.collname[0]"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(fixed("c"))
                    .from(fixed("t"))
                    .order_by_expr(Expr::col(fixed("c")).collate(n_(n)), Order::Asc))
            },
        },
        Site {
            id: "query/expr.as-enum.type",
            api: "Expr::as_enum(Name)",
            kinds: &["TypeCast.type_name.names[0]"],
            policy: TypePart,
            render: |n| sql(Query::select().expr(Expr::val("a").as_enum(n_(n)))),
        },
        // -- WITH ----------------------------------------------------------
        Site {
            id: "query/with.cte.name",
            api: "CommonTableExpression::new(Name, query)",
            kinds: &["CommonTableExpr.ctename"],
            policy: Quoted,
            render: |n| {
                let cte =
                    CommonTableExpression::new(n_(n), Query::select().expr(Expr::val(1)).take());
                sql(Query::select()
                    .column(Asterisk)
                    .from(fixed("t"))
                    .with(WithClause::new(cte)))
            },
        },
        Site {
            id: "query/with.cte.column",
            api: "CommonTableExpression::column(Name)",
            kinds: &["CommonTableExpr.aliascolnames[0]"],
            policy: Quoted,
            render: |n| {
                let mut cte = CommonTableExpression::new(
                    fixed("w"),
                    Query::select().expr(Expr::val(1)).take(),
                );
                cte.column(n_(n));
                sql(Query::select()
                    .column(Asterisk)
                    .from(fixed("t"))
                    .with(WithClause::new(cte)))
            },
        },
        Site {
            id: "query/with.recursive.search-set",
            api: "Search::new(order, expr, Name)",
            kinds: &["CommonTableExpr.search_clause.search_seq_column"],
            policy: Quoted,
            render: |n| {
                let mut clause = RecursiveWithClause::new(recursive_cte());
                clause.search(Search::new(
                    SearchOrder::DEPTH,
                    Expr::col(fixed("c")),
                    n_(n),
                ));
                sql(Query::select()
                    .column(Asterisk)
                    .from(fixed("w"))
                    .with(clause))
            },
        },
        Site {
            id: "query/with.recursive.cycle-set",
            api: "Cycle::new(expr, Name, using)",
            kinds: &["CommonTableExpr.cycle_clause.cycle_mark_column"],
            policy: Quoted,
            render: |n| {
                let mut clause = RecursiveWithClause::new(recursive_cte());
                clause.cycle(Cycle::new(Expr::col(fixed("c")), n_(n), fixed("p")));
                sql(Query::select()
                    .column(Asterisk)
                    .from(fixed("w"))
                    .with(clause))
            },
        },
        Site {
            id: "query/with.recursive.cycle-using",
            api: "Cycle::new(expr, set, Name)",
            kinds: &["CommonTableExpr.cycle_clause.cycle_path_column"],
            policy: Quoted,
            render: |n| {
                let mut clause = RecursiveWithClause::new(recursive_cte());
                clause.cycle(Cycle::new(Expr::col(fixed("c")), fixed("m"), n_(n)));
                sql(Query::select()
                    .column(Asterisk)
                    .from(fixed("w"))
                    .with(clause))
            },
        },
        // -- INSERT --------------------------------------------------------
        Site {
            id: "query/insert.table",
            api: "InsertStatement::into_table(Name)",
            kinds: &["InsertStmt.relation.relname"],
            policy: Quoted,
            render: |n| {
                sql(Query::insert()
                    .into_table(n_(n))
                    .columns([fixed("c")])
                    .values_panic([1.into()]))
            },
        },
        Site {
            id: "query/insert.table-alias",
            api: "InsertStatement::into_table(NamedTable::alias(Name))",
            kinds: &["InsertStmt.relation.alias.aliasname"],
            policy: Quoted,
            render: |n| {
                sql(Query::insert()
                    .into_table(fixed("t").into_named_table().alias(n_(n)))
                    .columns([fixed("c")])
                    .values_panic([1.into()]))
            },
        },
        Site {
            id: "query/insert.column",
            api: "InsertStatement::columns([Name])",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::insert()
                    .into_table(fixed("t"))
                    .columns([n_(n)])
                    .values_panic([1.into()]))
            },
        },
        Site {
            id: "query/insert.on-conflict.target",
            api: "OnConflict::column(Name)",
            kinds: &["IndexElem.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::insert()
                    .into_table(fixed("t"))
                    .columns([fixed("c")])
                    .values_panic([1.into()])
                    .on_conflict(OnConflict::column(n_(n)).do_nothing()))
            },
        },
        Site {
            id: "query/insert.on-conflict.target-key",
            api: "OnConflict::columns((Name, Name))",
            kinds: &["IndexElem.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::insert()
                    .into_table(fixed("t"))
                    .columns([fixed("c")])
                    .values_panic([1.into()])
                    .on_conflict(OnConflict::columns((fixed("c"), n_(n))).do_nothing()))
            },
        },
        Site {
            id: "query/insert.on-conflict.update-column",
            api: "ConflictTarget::update_column(Name)",
            kinds: &["ResTarget.name", "ColumnRef.fields[1]"],
            policy: Quoted,
            render: |n| {
                sql(Query::insert()
                    .into_table(fixed("t"))
                    .columns([fixed("c")])
                    .values_panic([1.into()])
                    .on_conflict(OnConflict::column(fixed("c")).update_column(n_(n))))
            },
        },
        Site {
            id: "query/insert.on-conflict.value",
            api: "ConflictTarget::value(Name, expr)",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::insert()
                    .into_table(fixed("t"))
                    .columns([fixed("c")])
                    .values_panic([1.into()])
                    .on_conflict(OnConflict::column(fixed("c")).value(n_(n), 2)))
            },
        },
        Site {
            id: "query/insert.on-conflict.constraint",
            api: "OnConflict::constraint(Name)",
            kinds: &["InsertStmt.on_conflict_clause.infer.conname"],
            policy: Quoted,
            render: |n| {
                sql(Query::insert()
                    .into_table(fixed("t"))
                    .columns([fixed("c")])
                    .values_panic([1.into()])
                    .on_conflict(OnConflict::constraint(n_(n)).do_nothing()))
            },
        },
        Site {
            id: "query/insert.on-conflict.constraint-update-column",
            api: "ConflictConstraint::update_column(Name)",
            kinds: &["ResTarget.name", "ColumnRef.fields[1]"],
            policy: Quoted,
            render: |n| {
                sql(Query::insert()
                    .into_table(fixed("t"))
                    .columns([fixed("c")])
                    .values_panic([1.into()])
                    .on_conflict(OnConflict::constraint(fixed("k")).update_column(n_(n))))
            },
        },
        Site {
            id: "query/insert.on-conflict.constraint-value",
            api: "ConflictConstraint::value(Name, expr)",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::insert()
                    .into_table(fixed("t"))
                    .columns([fixed("c")])
                    .values_panic([1.into()])
                    .on_conflict(OnConflict::constraint(fixed("k")).value(n_(n), 2)))
            },
        },
        Site {
            id: "query/insert.returning",
            api: "InsertStatement::returning_col(Name)",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| {
                sql(Query::insert()
                    .into_table(fixed("t"))
                    .columns([fixed("c")])
                    .values_panic([1.into()])
                    .returning_col(n_(n)))
            },
        },
        // -- UPDATE --------------------------------------------------------
        Site {
            id: "query/update.table",
            api: "UpdateStatement::table(Name)",
            kinds: &["UpdateStmt.relation.relname"],
            policy: Quoted,
            render: |n| sql(Query::update().table(n_(n)).value(fixed("c"), 1)),
        },
        Site {
            id: "query/update.set-column",
            api: "UpdateStatement::value(Name, value)",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| sql(Query::update().table(fixed("t")).value(n_(n), 1)),
        },
        Site {
            id: "query/update.from",
            api: "UpdateStatement::from(Name)",
            kinds: &["RangeVar.relname"],
            policy: Quoted,
            render: |n| {
                sql(Query::update()
                    .table(fixed("t"))
                    .value(fixed("c"), 1)
                    .from(n_(n)))
            },
        },
        Site {
            id: "query/update.returning",
            api: "UpdateStatement::returning_col(Name)",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| {
                sql(Query::update()
                    .table(fixed("t"))
                    .value(fixed("c"), 1)
                    .returning_col(n_(n)))
            },
        },
        Site {
            id: "query/returning.old-as",
            api: "ReturningClause::old_as(Name)",
            kinds: &["ReturningOption.value"],
            policy: Quoted,
            render: |n| {
                sql(Query::update()
                    .table(fixed("t"))
                    .value(fixed("c"), 1)
                    .returning(Query::returning().all().old_as(n_(n))))
            },
        },
        Site {
            id: "query/returning.new-as",
            api: "ReturningClause::new_as(Name)",
            kinds: &["ReturningOption.value"],
            policy: Quoted,
            render: |n| {
                sql(Query::update()
                    .table(fixed("t"))
                    .value(fixed("c"), 1)
                    .returning(Query::returning().all().new_as(n_(n))))
            },
        },
        Site {
            id: "query/returning.row-column",
            api: "Returning::column((ReturningRow, Name)) — the column part",
            kinds: &["ColumnRef.fields[1]"],
            policy: Quoted,
            render: |n| {
                sql(Query::update()
                    .table(fixed("t"))
                    .value(fixed("c"), 1)
                    .returning(Query::returning().column((ReturningRow::Old, n_(n)))))
            },
        },
        Site {
            id: "query/returning.expr-as",
            api: "Returning::expr_as(expr, Name) / exprs_as — the result column's name",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::update()
                    .table(fixed("t"))
                    .value(fixed("c"), 1)
                    .returning(Query::returning().expr_as(Expr::col(fixed("c")), n_(n))))
            },
        },
        // -- DELETE --------------------------------------------------------
        Site {
            id: "query/delete.table",
            api: "DeleteStatement::from_table(Name)",
            kinds: &["DeleteStmt.relation.relname"],
            policy: Quoted,
            render: |n| sql(Query::delete().from_table(n_(n))),
        },
        Site {
            id: "query/delete.using",
            api: "DeleteStatement::using(Name)",
            kinds: &["RangeVar.relname"],
            policy: Quoted,
            render: |n| sql(Query::delete().from_table(fixed("t")).using(n_(n))),
        },
        Site {
            id: "query/delete.returning",
            api: "DeleteStatement::returning_col(Name)",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| sql(Query::delete().from_table(fixed("t")).returning_col(n_(n))),
        },
        // -- MERGE ---------------------------------------------------------
        Site {
            id: "query/merge.target",
            api: "Query::merge(Name, ..)",
            kinds: &["MergeStmt.relation.relname"],
            policy: Quoted,
            render: |n| {
                sql(&Query::merge(n_(n), fixed("s"), merge_on())
                    .when_matched(MatchedAction::Delete))
            },
        },
        Site {
            id: "query/merge.target.schema",
            api: "Query::merge((Name, Name), ..) — the schema part",
            kinds: &["MergeStmt.relation.schemaname"],
            policy: Quoted,
            render: |n| {
                sql(&Query::merge((n_(n), fixed("t")), fixed("s"), merge_on())
                    .when_matched(MatchedAction::Delete))
            },
        },
        Site {
            id: "query/merge.target-alias",
            api: "Query::merge(NamedTable::alias(Name), ..)",
            kinds: &["MergeStmt.relation.alias.aliasname"],
            policy: Quoted,
            render: |n| {
                sql(&Query::merge(
                    fixed("t").into_named_table().alias(n_(n)),
                    fixed("s"),
                    merge_on(),
                )
                .when_matched(MatchedAction::Delete))
            },
        },
        Site {
            id: "query/merge.source",
            api: "Query::merge(.., Name, ..)",
            kinds: &["RangeVar.relname"],
            policy: Quoted,
            render: |n| {
                sql(&Query::merge(fixed("t"), n_(n), merge_on())
                    .when_matched(MatchedAction::Delete))
            },
        },
        Site {
            id: "query/merge.update.value",
            api: "MergeUpdate::value(Name, expr)",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| {
                sql(&Query::merge(fixed("t"), fixed("s"), merge_on())
                    .when_matched(MergeUpdate::value(n_(n), 1)))
            },
        },
        Site {
            id: "query/merge.update.and-value",
            api: "MergeUpdate::and_value(Name, expr)",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| {
                sql(&Query::merge(fixed("t"), fixed("s"), merge_on())
                    .when_matched(MergeUpdate::value(fixed("c"), 1).and_value(n_(n), 2)))
            },
        },
        Site {
            id: "query/merge.update.and-values",
            api: "MergeUpdate::and_values([(Name, expr)])",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| {
                sql(
                    &Query::merge(fixed("t"), fixed("s"), merge_on()).when_matched(
                        MergeUpdate::value(fixed("c"), 1)
                            .and_values([(n_(n), Expr::val(2).into())]),
                    ),
                )
            },
        },
        Site {
            id: "query/merge.insert.value",
            api: "MergeInsert::value(Name, expr)",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| {
                sql(&Query::merge(fixed("t"), fixed("s"), merge_on())
                    .when_not_matched(MergeInsert::value(n_(n), 1)))
            },
        },
        Site {
            id: "query/merge.insert.and-value",
            api: "MergeInsert::and_value(Name, expr)",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| {
                sql(&Query::merge(fixed("t"), fixed("s"), merge_on())
                    .when_not_matched(MergeInsert::value(fixed("c"), 1).and_value(n_(n), 2)))
            },
        },
        Site {
            id: "query/merge.insert.and-values",
            api: "MergeInsert::and_values([(Name, expr)])",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| {
                sql(
                    &Query::merge(fixed("t"), fixed("s"), merge_on()).when_not_matched(
                        MergeInsert::value(fixed("c"), 1)
                            .and_values([(n_(n), Expr::val(2).into())]),
                    ),
                )
            },
        },
        // -- SQL/JSON ---------------------------------------------------------
        Site {
            id: "query/json-exists.passing",
            api: "JsonExists::passing(value, Name)",
            kinds: &["JsonArgument.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .expr(Func::json_exists(Expr::col(fixed("c")), "$").passing(1, n_(n))))
            },
        },
        Site {
            id: "query/json-value.passing",
            api: "JsonValue::passing(value, Name)",
            kinds: &["JsonArgument.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .expr(Func::json_value(Expr::col(fixed("c")), "$").passing(1, n_(n))))
            },
        },
        Site {
            id: "query/json-query.passing",
            api: "JsonQuery::passing(value, Name)",
            kinds: &["JsonArgument.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .expr(Func::json_query(Expr::col(fixed("c")), "$").passing(1, n_(n))))
            },
        },
        Site {
            id: "query/json-table.alias",
            api: "JsonTable::alias(Name)",
            kinds: &["JsonTable.alias.aliasname"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(Asterisk)
                    .from(json_table().alias(n_(n))))
            },
        },
        Site {
            id: "query/json-table.path-name",
            api: "JsonTable::path_name(Name)",
            kinds: &["JsonTable.pathspec.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(Asterisk)
                    .from(json_table().path_name(n_(n)).alias(fixed("j"))))
            },
        },
        Site {
            id: "query/json-table.passing",
            api: "JsonTable::passing(value, Name)",
            kinds: &["JsonArgument.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(Asterisk)
                    .from(json_table().passing(1, n_(n)).alias(fixed("j"))))
            },
        },
        Site {
            id: "query/json-table.column.ordinality",
            api: "JsonTableColumn::ordinality(Name)",
            kinds: &["JsonTableColumn.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::select()
                    .column(Asterisk)
                    .from(json_table_of(JsonTableColumn::ordinality(n_(n)))))
            },
        },
        Site {
            id: "query/json-table.column.value",
            api: "JsonTableColumn::value(Name, type)",
            kinds: &["JsonTableColumn.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::select().column(Asterisk).from(json_table_of(
                    JsonTableColumn::value(n_(n), ColumnType::Text).into(),
                )))
            },
        },
        Site {
            id: "query/json-table.column.query",
            api: "JsonTableColumn::query(Name, type)",
            kinds: &["JsonTableColumn.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::select().column(Asterisk).from(json_table_of(
                    JsonTableColumn::query(n_(n), ColumnType::JsonBinary).into(),
                )))
            },
        },
        Site {
            id: "query/json-table.column.exists",
            api: "JsonTableColumn::exists(Name, type)",
            kinds: &["JsonTableColumn.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::select().column(Asterisk).from(json_table_of(
                    JsonTableColumn::exists(n_(n), ColumnType::Boolean).into(),
                )))
            },
        },
        Site {
            id: "query/json-table.nested.path-name",
            api: "JsonNestedColumns::path_name(Name)",
            kinds: &["JsonTableColumn.pathspec.name"],
            policy: Quoted,
            render: |n| {
                sql(Query::select().column(Asterisk).from(json_table_of(
                    JsonTableColumn::nested("$", JsonTableColumn::ordinality(fixed("o")))
                        .path_name(n_(n))
                        .into(),
                )))
            },
        },
    ]
}

/// The join condition of the MERGE sites, over fixed names only, so the name
/// under test is the one thing in the statement that moves.
fn merge_on() -> pgorm::pgorm_query::SimpleExpr {
    Expr::col((fixed("t"), fixed("c"))).equals((fixed("s"), fixed("c")))
}

/// A recursive CTE named `w` over a fixed column `c`, for the SEARCH and
/// CYCLE sites.
fn recursive_cte() -> CommonTableExpression {
    let mut cte = CommonTableExpression::new(
        fixed("w"),
        Query::select()
            .column(fixed("c"))
            .from(fixed("t"))
            .union(
                pgorm::pgorm_query::UnionType::All,
                Query::select().column(fixed("c")).from(fixed("w")).take(),
            )
            .take(),
    );
    cte.column(fixed("c"));
    cte
}

/// A `JSON_TABLE` over fixed names only, for the sites that name its alias,
/// its root path or a variable.
fn json_table() -> JsonTable {
    Func::json_table(
        Expr::col(fixed("c")),
        "$",
        JsonTableColumn::ordinality(fixed("o")),
    )
}

/// A `JSON_TABLE` aliased `j` whose one column is `column`.
fn json_table_of(column: JsonTableColumn) -> FromItem {
    Func::json_table(Expr::col(fixed("c")), "$", column).alias(fixed("j"))
}
