use super::*;
use crate::oracle::{assert_eq, assert_rejected, parsed_nodes};
use pgsql_syntax::protobuf::{DropBehavior, ObjectType, ViewCheckOption as ParsedCheckOption};

fn n(name: &str) -> Name {
    Name::runtime(name)
}

/// The one `kind` node in `sql`, as PostgreSQL's parser read it.
fn statement(sql: &str, kind: &str) -> serde_json::Value {
    let mut found = parsed_nodes(sql, kind);
    assert_eq!(found.len(), 1, "exactly one {kind} in {sql}");
    found.remove(0)
}

/// A list of `String` nodes, as the plain strings they hold.
fn strings(list: &serde_json::Value) -> Vec<&str> {
    list.as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|item| item["String"]["sval"].as_str().expect("a name"))
        .collect()
}

/// The names of a definition's `WITH (...)` options, in order.
fn option_names(node: &serde_json::Value) -> Vec<&str> {
    node["options"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|option| option["DefElem"]["defname"].as_str().expect("an option"))
        .collect()
}

/// `SELECT "id", "name" FROM "account" WHERE "active" = TRUE AND "name" <>
/// E'it\'s'`: a body carrying two values, one of them a string whose quote
/// the literal escapes.
fn view_body() -> SelectStatement {
    Query::select()
        .columns([n("id"), n("name")])
        .from(n("account"))
        .and_where(Expr::col(n("active")).eq(true))
        .and_where(Expr::col(n("name")).ne("it's"))
        .take()
}

const ACTIVE_ACCOUNTS: &str =
    r#"SELECT "id", "name" FROM "account" WHERE "active" = TRUE AND "name" <> E'it\'s'"#;

// [spec:pgorm:req:sql.ddl.view+1/test]    the name and the body are all a view needs, and the
// body's values are written into it as literals, there being nothing to bind them to
#[test]
fn a_view_writes_its_body_with_values_inlined() -> error::Result<()> {
    let sql = View::create(n("active_account"), view_body())?.to_string();
    assert_eq!(
        sql,
        format!(r#"CREATE VIEW "active_account" AS {ACTIVE_ACCOUNTS}"#)
    );
    assert!(!sql.contains('$'), "{sql}");
    let view = statement(&sql, "ViewStmt");
    assert_eq!(view["view"]["relname"], "active_account");
    assert_eq!(view["replace"], false);
    assert!(strings(&view["aliases"]).is_empty());
    assert!(option_names(&view).is_empty());
    assert_eq!(
        view["with_check_option"],
        ParsedCheckOption::NoCheckOption as i32
    );
    assert_eq!(
        parsed_nodes(&sql, "AConst")
            .iter()
            .filter_map(|constant| constant["val"]["Sval"]["sval"].as_str())
            .collect::<Vec<_>>(),
        ["it's"]
    );
    Ok(())
}

// [spec:pgorm:req:sql.ddl.view+1/test]    every clause of a plain view, read back by the grammar
// as the clause it was written as
#[test]
fn every_view_clause_renders_in_one_order() -> error::Result<()> {
    let sql = View::create((n("app"), n("active_account")), view_body())?
        .check_option(ViewCheckOption::Local)
        .security_invoker()
        .columns([n("account_id"), n("account_name")])
        .security_barrier()
        .or_replace()
        .to_string();
    assert_eq!(
        sql,
        [
            r#"CREATE OR REPLACE VIEW "app"."active_account" ("account_id", "account_name")"#,
            "WITH (security_barrier, security_invoker) AS",
            ACTIVE_ACCOUNTS,
            "WITH LOCAL CHECK OPTION",
        ]
        .join(" ")
    );
    let view = statement(&sql, "ViewStmt");
    assert_eq!(view["replace"], true);
    assert_eq!(view["view"]["schemaname"], "app");
    assert_eq!(view["view"]["relname"], "active_account");
    assert_eq!(strings(&view["aliases"]), ["account_id", "account_name"]);
    assert_eq!(
        option_names(&view),
        ["security_barrier", "security_invoker"]
    );
    assert_eq!(
        view["with_check_option"],
        ParsedCheckOption::LocalCheckOption as i32
    );

    let one_option = View::create(n("v"), Query::select().expr(1).take())?
        .security_invoker()
        .to_string();
    assert_eq!(
        one_option,
        r#"CREATE VIEW "v" WITH (security_invoker) AS SELECT 1"#
    );
    Ok(())
}

// [spec:pgorm:req:sql.ddl.view+1/test]    the check option is one slot, LOCAL or CASCADED, the
// later replacing the earlier
#[test]
fn a_check_option_is_local_or_cascaded() -> error::Result<()> {
    let view = || View::create(n("v"), Query::select().expr(1).take());
    for (option, words, parsed) in [
        (
            ViewCheckOption::Local,
            "LOCAL",
            ParsedCheckOption::LocalCheckOption,
        ),
        (
            ViewCheckOption::Cascaded,
            "CASCADED",
            ParsedCheckOption::CascadedCheckOption,
        ),
    ] {
        let sql = view()?
            .check_option(ViewCheckOption::Local)
            .check_option(ViewCheckOption::Cascaded)
            .check_option(option)
            .to_string();
        assert_eq!(
            sql,
            format!(r#"CREATE VIEW "v" AS SELECT 1 WITH {words} CHECK OPTION"#)
        );
        assert_eq!(
            statement(&sql, "ViewStmt")["with_check_option"],
            parsed as i32
        );
        let mut built = view()?;
        built.check_option(option);
        assert_eq!(built.get_check_option(), Some(option));
    }
    assert_eq!(view()?.get_check_option(), None);
    Ok(())
}

/// `SELECT 1 UNION ALL SELECT "n" + 1 FROM "counter" WHERE "n" < 3`, which
/// reads the view it defines.
fn counting() -> SelectStatement {
    Query::select()
        .expr(1)
        .union(
            UnionType::All,
            Query::select()
                .expr(Expr::col(n("n")).add(1))
                .from(n("counter"))
                .and_where(Expr::col(n("n")).lt(3))
                .take(),
        )
        .take()
}

// [spec:pgorm:req:sql.ddl.view+1/test]    a recursive view always names its columns, which the
// grammar turns into the WITH RECURSIVE query it defines the view as
#[test]
fn a_recursive_view_names_its_columns() -> error::Result<()> {
    let sql = View::create_recursive((n("app"), n("counter")), n("n"), counting())?
        .column(n("depth"))
        .columns([n("path")])
        .or_replace()
        .security_barrier()
        .to_string();
    assert_eq!(
        sql,
        [
            r#"CREATE OR REPLACE RECURSIVE VIEW "app"."counter" ("n", "depth", "path")"#,
            r#"WITH (security_barrier) AS"#,
            r#"SELECT 1 UNION ALL (SELECT "n" + 1 FROM "counter" WHERE "n" < 3)"#,
        ]
        .join(" ")
    );
    let view = statement(&sql, "ViewStmt");
    assert_eq!(view["replace"], true);
    assert_eq!(option_names(&view), ["security_barrier"]);
    let with = &view["query"]["SelectStmt"]["with_clause"];
    assert_eq!(with["recursive"], true);
    let cte = &with["ctes"][0]["CommonTableExpr"];
    assert_eq!(cte["ctename"], "counter");
    assert_eq!(strings(&cte["aliascolnames"]), ["n", "depth", "path"]);

    let mut plain = View::create_recursive(n("counter"), n("n"), counting())?;
    assert_eq!(plain.get_columns(), [n("n")]);
    plain.column(n("m"));
    assert_eq!(plain.get_columns().len(), 2);
    Ok(())
}

// [spec:pgorm:req:sql.ddl.view+1/test]    what the grammar refuses the builder has no method or
// constructor for: a recursive view without columns or with a check option, and a view IF
// NOT EXISTS
#[test]
fn the_grammar_refuses_what_a_view_cannot_build() {
    assert_rejected(r#"CREATE RECURSIVE VIEW "counter" AS SELECT 1"#);
    let refused = assert_rejected(
        r#"CREATE RECURSIVE VIEW "counter" ("n") AS SELECT 1 WITH LOCAL CHECK OPTION"#,
    );
    assert!(
        refused.contains("WITH CHECK OPTION not supported"),
        "{refused}"
    );
    assert_rejected(r#"CREATE VIEW IF NOT EXISTS "v" AS SELECT 1"#);
    assert_rejected(r#"CREATE VIEW "v" () AS SELECT 1"#);
}

// [spec:pgorm:req:sql.ddl.view+1/test]    a view drop names every view it drops, and one behavior
#[test]
fn a_view_drop_names_one_or_more_views() {
    assert_eq!(View::drop(n("a")).to_string(), r#"DROP VIEW "a""#);

    let mut drop = View::drop(n("a"));
    drop.name((n("app"), n("b")))
        .if_exists()
        .cascade()
        .restrict();
    let sql = drop.to_string();
    assert_eq!(sql, r#"DROP VIEW IF EXISTS "a", "app"."b" RESTRICT"#);
    assert_eq!(drop.names().count(), 2);
    let read = statement(&sql, "DropStmt");
    assert_eq!(read["remove_type"], ObjectType::ObjectView as i32);
    assert_eq!(read["behavior"], DropBehavior::DropRestrict as i32);
    assert_eq!(read["missing_ok"], true);

    let sql = View::drop(n("a")).restrict().cascade().to_string();
    assert_eq!(sql, r#"DROP VIEW "a" CASCADE"#);
}

// [spec:pgorm:req:sql.ddl.materialized-view+2/test]    every clause of a materialized view,
// read back as the CREATE TABLE AS the grammar holds it as
#[test]
fn a_materialized_view_is_created_empty_or_full() -> error::Result<()> {
    let sql = MaterializedView::create(n("active_account"), view_body())?.to_string();
    assert_eq!(
        sql,
        format!(r#"CREATE MATERIALIZED VIEW "active_account" AS {ACTIVE_ACCOUNTS}"#)
    );
    let create = statement(&sql, "CreateTableAsStmt");
    assert_eq!(create["objtype"], ObjectType::ObjectMatview as i32);
    assert_eq!(create["into"]["rel"]["relname"], "active_account");
    assert_eq!(create["into"]["skip_data"], false);

    let mut statement_ = MaterializedView::create((n("app"), n("active_account")), view_body())?;
    statement_
        .with_no_data()
        .columns([n("account_id")])
        .column(n("account_name"))
        .if_not_exists();
    assert_eq!(statement_.get_columns().len(), 2);
    let sql = statement_.to_string();
    assert_eq!(
        sql,
        [
            r#"CREATE MATERIALIZED VIEW IF NOT EXISTS "app"."active_account""#,
            r#"("account_id", "account_name") AS"#,
            ACTIVE_ACCOUNTS,
            "WITH NO DATA",
        ]
        .join(" ")
    );
    let create = statement(&sql, "CreateTableAsStmt");
    assert_eq!(create["if_not_exists"], true);
    assert_eq!(create["into"]["rel"]["schemaname"], "app");
    assert_eq!(
        strings(&create["into"]["col_names"]),
        ["account_id", "account_name"]
    );
    assert_eq!(create["into"]["skip_data"], true);

    assert_rejected(r#"CREATE OR REPLACE MATERIALIZED VIEW "m" AS SELECT 1"#);
    Ok(())
}

// [spec:pgorm:req:sql.ddl.materialized-view+2/test]    CONCURRENTLY and WITH NO DATA are one
// choice, the later winning, so the pair the server refuses (42601) and the grammar
// accepts has no render
#[test]
fn a_refresh_is_concurrent_or_empty_never_both() {
    let refresh = || MaterializedView::refresh((n("report"), n("daily")));
    let sql = refresh().to_string();
    assert_eq!(sql, r#"REFRESH MATERIALIZED VIEW "report"."daily""#);
    let read = statement(&sql, "RefreshMatViewStmt");
    assert_eq!(read["relation"]["relname"], "daily");
    assert_eq!(read["concurrent"], false);
    assert_eq!(read["skip_data"], false);

    let mut concurrent = refresh();
    concurrent.with_no_data().concurrently();
    assert!(concurrent.is_concurrently() && !concurrent.is_with_no_data());
    let sql = concurrent.to_string();
    assert_eq!(
        sql,
        r#"REFRESH MATERIALIZED VIEW CONCURRENTLY "report"."daily""#
    );
    assert_eq!(statement(&sql, "RefreshMatViewStmt")["concurrent"], true);

    let mut empty = refresh();
    empty.concurrently().with_no_data();
    assert!(empty.is_with_no_data() && !empty.is_concurrently());
    let sql = empty.to_string();
    assert_eq!(
        sql,
        r#"REFRESH MATERIALIZED VIEW "report"."daily" WITH NO DATA"#
    );
    let read = statement(&sql, "RefreshMatViewStmt");
    assert_eq!(read["skip_data"], true);
    assert_eq!(read["concurrent"], false);

    // The grammar takes both, which is why the slot, and not the oracle, is
    // what keeps the pair out.
    crate::oracle::assert_parses(r#"REFRESH MATERIALIZED VIEW CONCURRENTLY "m" WITH NO DATA"#);
}

// [spec:pgorm:req:sql.ddl.materialized-view+2/test]    a materialized view drop is a drop of
// its own kind
#[test]
fn a_materialized_view_drop_names_its_kind() {
    let mut drop = MaterializedView::drop(n("a"));
    drop.name(n("b")).if_exists().cascade();
    let sql = drop.to_string();
    assert_eq!(sql, r#"DROP MATERIALIZED VIEW IF EXISTS "a", "b" CASCADE"#);
    assert_eq!(drop.names().count(), 2);
    let read = statement(&sql, "DropStmt");
    assert_eq!(read["remove_type"], ObjectType::ObjectMatview as i32);
    assert_eq!(read["behavior"], DropBehavior::DropCascade as i32);
}

// [spec:pgorm:req:sql.ddl.create-table-as+1/test]    CREATE TABLE AS takes the query's columns,
// renamed or not, and its rows or none
#[test]
fn a_table_is_created_from_a_query() -> error::Result<()> {
    let sql = Table::create_as(n("account_copy"), view_body())?.to_string();
    assert_eq!(
        sql,
        format!(r#"CREATE TABLE "account_copy" AS {ACTIVE_ACCOUNTS}"#)
    );
    let create = statement(&sql, "CreateTableAsStmt");
    assert_eq!(create["objtype"], ObjectType::ObjectTable as i32);
    assert_eq!(create["into"]["rel"]["relname"], "account_copy");

    let mut statement_ = Table::create_as((n("app"), n("account_copy")), view_body())?;
    statement_
        .column(n("account_id"))
        .with_no_data()
        .if_not_exists();
    assert_eq!(statement_.get_columns(), [n("account_id")]);
    let sql = statement_.to_string();
    assert_eq!(
        sql,
        format!(
            r#"CREATE TABLE IF NOT EXISTS "app"."account_copy" ("account_id") AS {ACTIVE_ACCOUNTS} WITH NO DATA"#
        )
    );
    let create = statement(&sql, "CreateTableAsStmt");
    assert_eq!(create["objtype"], ObjectType::ObjectTable as i32);
    assert_eq!(create["if_not_exists"], true);
    assert_eq!(strings(&create["into"]["col_names"]), ["account_id"]);
    assert_eq!(create["into"]["skip_data"], true);
    Ok(())
}

/// Hold `create` to the checks a stored definition's query passes as it is
/// taken: a tie limit with no ORDER BY, one beside SKIP LOCKED, and a
/// projection over a window no WINDOW clause declares are each refused with
/// pgorm-query's error, and the same queries ordered, unlocked and declared
/// construct and render after `AS`.
fn assert_body_checked<S: ToString>(create: impl Fn(SelectStatement) -> error::Result<S>) {
    let tied = Query::select()
        .column(n("id"))
        .from(n("account"))
        .limit_with_ties(2)
        .take();
    assert_eq!(
        create(tied.clone()).err(),
        Some(error::Error::TiesWithoutOrder)
    );
    let ordered = tied.clone().order_by(n("id"), Order::Asc).take();
    let skipping = ordered
        .clone()
        .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
        .take();
    assert_eq!(
        create(skipping).err(),
        Some(error::Error::TiesWithSkipLocked)
    );

    let windowed = Query::select()
        .expr_window_name(Func::count(Expr::col(n("id"))), n("w"))
        .from(n("account"))
        .take();
    assert_eq!(
        create(windowed.clone()).err(),
        Some(error::Error::WindowUndefined {
            name: "w".to_owned()
        })
    );
    let declared = windowed
        .clone()
        .window(n("w"), WindowStatement::partition_by(n("name")))
        .take();

    for body in [ordered, declared] {
        let rendered = format!("AS {body}");
        let sql = create(body)
            .expect("a query the grammar takes constructs")
            .to_string();
        assert!(sql.contains(&rendered), "{sql}");
    }
}

// [spec:pgorm:req:sql.ddl.view+1/test]    both view constructors refuse a body whose tie limit
// or WINDOW clause the grammar refuses, before any statement exists
#[test]
fn a_view_refuses_a_body_the_grammar_refuses() {
    assert_body_checked(|body| View::create(n("v"), body));
    assert_body_checked(|body| View::create_recursive(n("v"), n("c"), body));

    // The parser refuses the unordered tie limit inside a definition as it
    // does on its own (`42601`), which is what the check stands in for.
    let tied = Query::select()
        .column(n("id"))
        .from(n("account"))
        .limit_with_ties(2)
        .take();
    assert_rejected(&format!(r#"CREATE VIEW "v" AS {tied}"#));
}

// [spec:pgorm:req:sql.ddl.materialized-view+2/test]    a materialized view's constructor
// refuses the body a view's refuses
#[test]
fn a_materialized_view_refuses_a_refused_body() {
    assert_body_checked(|body| MaterializedView::create(n("m"), body));
}

// [spec:pgorm:req:sql.ddl.create-table-as+1/test]    CREATE TABLE AS refuses the body a view's
// constructor refuses
#[test]
fn create_table_as_refuses_a_refused_body() {
    assert_body_checked(|body| Table::create_as(n("t"), body));
}
