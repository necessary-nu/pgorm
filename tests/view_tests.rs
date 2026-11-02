#![allow(unused_imports, dead_code)]

//! Views, materialized views and `CREATE TABLE AS` against a live PostgreSQL
//! server.
//!
//! The render tests in pgorm-query hold each statement to libpg_query's
//! `ViewStmt`, `CreateTableAsStmt`, `RefreshMatViewStmt` and `DropStmt`. What
//! only a server settles is what the statements mean: that an entity whose
//! table is a view reads the rows the view shows and writes through it to the
//! table beneath; that a check option refuses a row the view would hide, and
//! `LOCAL` and `CASCADED` differ in which views they consult; that the
//! security options change what a query over the view can learn and whose
//! privileges it reads with; that a materialized view holds its rows until it
//! is refreshed, and what a concurrent refresh needs; and what the server
//! refuses that no builder value records — a check option on a view that is
//! not updatable, an `OR REPLACE` that moves a column, a drop of the wrong
//! kind.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    ColumnDef, Expr, Func, Index, MaterializedView, Name, Order, Query, SelectStatement, Table,
    UnionType, View, ViewCheckOption, WindowStatement, error::Error as QueryError,
};
use pgorm::{ConnectionTrait, TransactionTrait, entity::prelude::*};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

/// An account the `active_account` view shows: the view's three columns, its
/// key the table's.
mod active_account {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "active_account")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
        pub active: bool,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// The same columns through `checked_account`, which carries a check option.
mod checked_account {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "checked_account")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
        pub active: bool,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A row of the `account_total` materialized view: how many accounts are
/// active and how many are not.
mod account_total {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "account_total")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub active: bool,
        pub total: i64,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

fn n(name: &str) -> Name {
    Name::runtime(name)
}

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

/// `account`: an identity key, a name and whether the account is active,
/// with two active accounts and one that is not.
async fn accounts(db: &DatabaseConnection) -> Result<(), Error> {
    let create = Table::create(n("account"))
        .col(ColumnDef::new(n("id")).integer().identity())
        .primary_key(n("id"))
        .col(ColumnDef::new(n("name")).text().not_null())
        .col(
            ColumnDef::new(n("active"))
                .boolean()
                .not_null()
                .default(true),
        )
        .to_string();
    db.batch_execute(&create).await?;
    db.batch_execute(
        "INSERT INTO account (name, active) VALUES ('ada', true), ('it''s', true), ('cy', false)",
    )
    .await
}

/// `SELECT "id", "name", "active" FROM "account" WHERE "active" = TRUE`.
fn active_accounts() -> SelectStatement {
    Query::select()
        .columns([n("id"), n("name"), n("active")])
        .from(n("account"))
        .and_where(Expr::col(n("active")).eq(true))
        .take()
}

/// Every `name` in `relation`, ordered by `id`; the relation may carry a
/// `WHERE` of its own.
async fn names(db: &DatabaseConnection, relation: &str) -> Result<Vec<String>, Error> {
    let rows = db
        .query_all(&format!("SELECT name FROM {relation} ORDER BY id"), &[])
        .await?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

/// An entity whose table is a view reads the rows the view shows, and an
/// insert, update and delete through the entity land in the table beneath,
/// its identity filling the key. The body's values — the `TRUE` and a quoted
/// string — are written into the definition, there being no parameter a view
/// could bind.
// [spec:pgorm:req:sql.ddl.view+1/test]    an entity reads through a view and writes through a
// simple auto-updatable one, and the body's values are stored inlined
#[pgorm_macros::test]
async fn an_entity_reads_and_writes_through_a_view() -> Result<(), Error> {
    let ctx = TestContext::new("view_entity_reads_writes").await;
    let db = ctx.db.get().await?;
    accounts(&db).await?;

    let create = View::create(n("active_account"), active_accounts())?.to_string();
    assert_eq!(
        create,
        r#"CREATE VIEW "active_account" AS SELECT "id", "name", "active" FROM "account" WHERE "active" = TRUE"#
    );
    db.batch_execute(&create).await?;

    let read = active_account::Entity::find()
        .order_by_asc(active_account::Column::Id)
        .all(&db)
        .await?;
    assert_eq!(
        read.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        ["ada", "it's"]
    );
    let found = active_account::Entity::find()
        .filter(active_account::Column::Name.eq("it's"))
        .one(&db)
        .await?;
    assert_eq!(found.id, 2);

    let inserted = active_account::ActiveModel {
        name: set("dee"),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    assert_eq!((inserted.id, inserted.active), (4, true));

    let updated = active_account::ActiveModel {
        id: Unchanged(inserted.id),
        name: set("dee2"),
        ..Default::default()
    }
    .update(&db)
    .await?;
    assert_eq!(updated.name, "dee2");
    assert_eq!(names(&db, "account").await?, ["ada", "it's", "cy", "dee2"]);

    active_account::Entity::delete_by_id(1).exec(&db).await?;
    assert_eq!(names(&db, "account").await?, ["it's", "cy", "dee2"]);
    // The view's condition bounds what a write through it reaches: the
    // inactive account is not a row of the view, so no delete finds it.
    let missed = active_account::Entity::delete_by_id(3).exec(&db).await?;
    assert_eq!(missed, 0);

    // A value in the body is a literal in the stored definition.
    let filtered = View::create(
        n("named_account"),
        Query::select()
            .column(n("id"))
            .from(n("account"))
            .and_where(Expr::col(n("name")).eq("it's"))
            .take(),
    )?
    .to_string();
    db.batch_execute(&filtered).await?;
    let row = db
        .query_one("SELECT pg_get_viewdef('named_account'::regclass)", &[])
        .await?;
    let definition: String = row.get(0);
    assert!(definition.contains("'it''s'::text"), "{definition}");
    assert_eq!(
        names(&db, "account WHERE id IN (SELECT id FROM named_account)").await?,
        ["it's"]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A check option refuses a row written through the view that the view would
/// not show (`44000`), from an entity as from SQL. `LOCAL` consults the
/// view's own condition alone and `CASCADED` every view beneath it too; and a
/// view the server cannot update through takes no check option (`0A000`).
// [spec:pgorm:req:sql.ddl.view+1/test]    a check option refuses a hidden row (44000), LOCAL
// and CASCADED differ in the views they consult, and a view that is not updatable refuses one
#[pgorm_macros::test]
async fn a_check_option_refuses_rows_the_view_hides() -> Result<(), Error> {
    let ctx = TestContext::new("view_check_option").await;
    let db = ctx.db.get().await?;
    accounts(&db).await?;

    let checked = View::create(n("checked_account"), active_accounts())?
        .check_option(ViewCheckOption::Cascaded)
        .to_string();
    db.batch_execute(&checked).await?;
    let admitted = checked_account::ActiveModel {
        name: set("ok"),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    assert!(admitted.active);
    let refused = checked_account::ActiveModel {
        name: set("hidden"),
        active: set(false),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect_err("a row the view would hide");
    refused_with(&refused, &SqlState::WITH_CHECK_OPTION_VIOLATION);
    let moved = checked_account::ActiveModel {
        id: Unchanged(admitted.id),
        active: set(false),
        ..Default::default()
    }
    .update(&db)
    .await
    .expect_err("an update moving a row out of the view");
    refused_with(&moved, &SqlState::WITH_CHECK_OPTION_VIOLATION);

    // Over a view with no option of its own, LOCAL holds a row to the upper
    // view's condition alone and CASCADED to the lower view's as well.
    db.batch_execute(&View::create(n("active_account"), active_accounts())?.to_string())
        .await?;
    let short = || {
        Query::select()
            .columns([n("id"), n("name"), n("active")])
            .from(n("active_account"))
            .and_where(Expr::expr(Func::char_length(Expr::col(n("name")))).lt(5))
            .take()
    };
    for (view, option) in [
        ("short_local", ViewCheckOption::Local),
        ("short_cascaded", ViewCheckOption::Cascaded),
    ] {
        let create = View::create(n(view), short())?
            .check_option(option)
            .to_string();
        db.batch_execute(&create).await?;
        let long = db
            .execute(
                &format!("INSERT INTO {view} (name) VALUES ('toolong')"),
                &[],
            )
            .await
            .expect_err("a name the upper view's condition refuses");
        refused_with(&long, &SqlState::WITH_CHECK_OPTION_VIOLATION);
    }
    db.execute(
        "INSERT INTO short_local (name, active) VALUES ('lo', false)",
        &[],
    )
    .await?;
    let cascaded = db
        .execute(
            "INSERT INTO short_cascaded (name, active) VALUES ('ca', false)",
            &[],
        )
        .await
        .expect_err("a row the lower view hides, under CASCADED");
    refused_with(&cascaded, &SqlState::WITH_CHECK_OPTION_VIOLATION);

    let aggregate = View::create(
        n("account_count"),
        Query::select()
            .expr(Func::count(Expr::col(n("id"))))
            .from(n("account"))
            .take(),
    )?
    .check_option(ViewCheckOption::Local)
    .to_string();
    let not_updatable = db
        .batch_execute(&aggregate)
        .await
        .expect_err("a check option on a view that is not updatable");
    refused_with(&not_updatable, &SqlState::FEATURE_NOT_SUPPORTED);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The options a view has, as the catalogue holds them.
async fn reloptions(db: &DatabaseConnection, relation: &str) -> Result<Vec<String>, Error> {
    let row = db
        .query_one(
            "SELECT coalesce(reloptions, '{}') FROM pg_class WHERE oid = $1::text::regclass",
            &[&relation],
        )
        .await?;
    Ok(row.get(0))
}

/// `OR REPLACE` may add columns after those a view has and nothing else
/// (`42P16`), and replaces the view's options with its own; the column names
/// rename the query's leading columns, and more names than columns are
/// refused (`42601`).
// [spec:pgorm:req:sql.ddl.view+1/test]    OR REPLACE adds columns at the end only (42P16) and
// resets what it leaves out; column names rename the leading columns, too many are refused
#[pgorm_macros::test]
async fn or_replace_keeps_columns_and_resets_options() -> Result<(), Error> {
    let ctx = TestContext::new("view_or_replace").await;
    let db = ctx.db.get().await?;
    accounts(&db).await?;

    let body = |columns: &[&str]| {
        Query::select()
            .columns(columns.iter().map(|c| n(c)))
            .from(n("account"))
            .take()
    };
    let create = View::create(n("account_view"), body(&["id"]))?
        .security_barrier()
        .check_option(ViewCheckOption::Local)
        .to_string();
    db.batch_execute(&create).await?;
    assert_eq!(
        reloptions(&db, "account_view").await?,
        ["security_barrier=true", "check_option=local"]
    );

    let widened = View::create(n("account_view"), body(&["id", "name"]))?
        .or_replace()
        .to_string();
    db.batch_execute(&widened).await?;
    assert!(reloptions(&db, "account_view").await?.is_empty());
    assert_eq!(names(&db, "account_view").await?, ["ada", "it's", "cy"]);

    for moved in [body(&["name", "id"]), body(&["id"])] {
        let replaced = View::create(n("account_view"), moved)?
            .or_replace()
            .to_string();
        let refused = db
            .batch_execute(&replaced)
            .await
            .expect_err("a replacement that moves or drops a column");
        refused_with(&refused, &SqlState::INVALID_TABLE_DEFINITION);
    }

    let renamed = View::create(n("renamed"), body(&["id", "name"]))?
        .column(n("account_id"))
        .to_string();
    db.batch_execute(&renamed).await?;
    let columns: Vec<String> = db
        .query_all(
            "SELECT attname::text FROM pg_attribute WHERE attrelid = 'renamed'::regclass \
             AND attnum > 0 ORDER BY attnum",
            &[],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(columns, ["account_id", "name"]);
    let too_many = View::create(n("too_many"), body(&["id"]))?
        .columns([n("a"), n("b")])
        .to_string();
    let refused = db
        .batch_execute(&too_many)
        .await
        .expect_err("more column names than columns");
    refused_with(&refused, &SqlState::SYNTAX_ERROR);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// `security_barrier` keeps a caller's cheap function from seeing the rows
/// the view's condition hides, and `security_invoker` checks the relations
/// the view reads against the querying role rather than the view's owner.
// [spec:pgorm:req:sql.ddl.view+1/test]    security_barrier keeps a hidden row from a caller's
// function, and security_invoker reads with the querying role's privileges (42501)
#[pgorm_macros::test]
async fn security_options_change_what_a_view_reveals() -> Result<(), Error> {
    let ctx = TestContext::new("view_security_options").await;
    let db = ctx.db.get().await?;
    accounts(&db).await?;

    // A function cheaper than the view's condition, which the planner runs
    // first unless a barrier says otherwise, and which fails on the row the
    // view hides.
    db.batch_execute(
        "CREATE FUNCTION peek(id integer) RETURNS boolean LANGUAGE plpgsql COST 0.0000001 AS \
         $$ BEGIN IF id = 3 THEN RAISE EXCEPTION 'saw the hidden row'; END IF; RETURN true; END $$",
    )
    .await?;
    let visible = || {
        Query::select()
            .columns([n("id"), n("name")])
            .from(n("account"))
            .and_where(Expr::col(n("id")).ne(3))
            .take()
    };
    db.batch_execute(&View::create(n("leaky"), visible())?.to_string())
        .await?;
    let barrier = View::create(n("barrier"), visible())?
        .security_barrier()
        .to_string();
    db.batch_execute(&barrier).await?;
    let leaked = db
        .query_all("SELECT id FROM leaky WHERE peek(id)", &[])
        .await
        .expect_err("the plain view lets the function see the hidden row");
    refused_with(&leaked, &SqlState::RAISE_EXCEPTION);
    assert_eq!(
        db.query_all("SELECT id FROM barrier WHERE peek(id) ORDER BY id", &[])
            .await?
            .iter()
            .map(|row| row.get::<_, i32>(0))
            .collect::<Vec<_>>(),
        [1, 2]
    );

    let role = scoped_db_name("view_tests_reader");
    db.batch_execute(&format!("CREATE ROLE \"{role}\"")).await?;
    db.batch_execute(&View::create(n("definer"), visible())?.to_string())
        .await?;
    let invoker = View::create(n("invoker"), visible())?
        .security_invoker()
        .to_string();
    db.batch_execute(&invoker).await?;
    assert_eq!(reloptions(&db, "invoker").await?, ["security_invoker=true"]);
    db.batch_execute(&format!(
        "GRANT SELECT ON definer, invoker TO \"{role}\"; SET ROLE \"{role}\""
    ))
    .await?;
    let through_owner = db.query_all("SELECT id FROM definer", &[]).await?;
    assert_eq!(through_owner.len(), 2);
    let as_invoker = db
        .query_all("SELECT id FROM invoker", &[])
        .await
        .expect_err("the querying role has no privilege on the table");
    refused_with(&as_invoker, &SqlState::INSUFFICIENT_PRIVILEGE);
    db.batch_execute(&format!(
        "RESET ROLE; DROP OWNED BY \"{role}\"; DROP ROLE \"{role}\""
    ))
    .await?;

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A recursive view reads itself by the columns it names.
// [spec:pgorm:req:sql.ddl.view+1/test]    a recursive view counts through its own rows
#[pgorm_macros::test]
async fn a_recursive_view_reads_itself() -> Result<(), Error> {
    let ctx = TestContext::new("view_recursive").await;
    let db = ctx.db.get().await?;

    let counter = View::create_recursive(
        n("counter"),
        n("n"),
        Query::select()
            .expr(1)
            .union(
                UnionType::All,
                Query::select()
                    .expr(Expr::col(n("n")).add(1))
                    .from(n("counter"))
                    .and_where(Expr::col(n("n")).lt(4))
                    .take(),
            )
            .take(),
    )?
    .to_string();
    db.batch_execute(&counter).await?;
    let counted: Vec<i32> = db
        .query_all("SELECT n FROM counter ORDER BY n", &[])
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(counted, [1, 2, 3, 4]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// What `account_total` holds, through its entity.
async fn totals(db: &DatabaseConnection) -> Result<Vec<(bool, i64)>, Error> {
    Ok(account_total::Entity::find()
        .order_by_asc(account_total::Column::Active)
        .all(db)
        .await?
        .into_iter()
        .map(|row| (row.active, row.total))
        .collect())
}

/// A materialized view holds the rows its query returned when it was last
/// populated: unpopulated, it refuses a read (`55000`); refreshed, it shows
/// the table as it was then, through an entity as through SQL, and refuses a
/// write (`42809`). A concurrent refresh needs a unique index (`55000`) and a
/// populated view (`0A000`), and runs inside a transaction block.
// [spec:pgorm:req:sql.ddl.materialized-view+1/test]    a materialized view holds its rows until
// a refresh, a concurrent refresh needs a unique index and data, and an entity reads it
#[pgorm_macros::test]
async fn a_materialized_view_holds_rows_until_refreshed() -> Result<(), Error> {
    let ctx = TestContext::new("view_materialized").await;
    let mut db = ctx.db.get().await?;
    accounts(&db).await?;

    let create = MaterializedView::create(
        n("account_total"),
        Query::select()
            .column(n("active"))
            .expr(Func::count(Expr::col(n("id"))))
            .from(n("account"))
            .group_by_col(n("active"))
            .take(),
    )?
    .columns([n("active"), n("total")])
    .with_no_data()
    .to_string();
    db.batch_execute(&create).await?;
    let unpopulated = account_total::Entity::find()
        .all(&db)
        .await
        .expect_err("a materialized view created WITH NO DATA");
    refused_with(&unpopulated, &SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE);
    let concurrent = MaterializedView::refresh(n("account_total"))
        .concurrently()
        .to_string();
    let not_populated = db
        .batch_execute(&concurrent)
        .await
        .expect_err("a concurrent refresh of an unpopulated view");
    refused_with(&not_populated, &SqlState::FEATURE_NOT_SUPPORTED);

    db.batch_execute(&MaterializedView::refresh(n("account_total")).to_string())
        .await?;
    assert_eq!(totals(&db).await?, [(false, 1), (true, 2)]);

    db.batch_execute("INSERT INTO account (name) VALUES ('new')")
        .await?;
    assert_eq!(totals(&db).await?, [(false, 1), (true, 2)]);

    let no_index = db
        .batch_execute(&concurrent)
        .await
        .expect_err("a concurrent refresh with no unique index");
    refused_with(&no_index, &SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE);
    let index = Index::create(n("account_total"), n("active"))
        .name(n("account_total_active"))
        .unique()
        .to_string();
    db.batch_execute(&index).await?;
    db.transaction(async |txn| txn.batch_execute(&concurrent).await)
        .await
        .expect("the concurrent refresh commits");
    assert_eq!(totals(&db).await?, [(false, 1), (true, 3)]);

    let written = account_total::ActiveModel {
        active: set(true),
        total: set(9),
    }
    .insert(&db)
    .await
    .expect_err("a write to a materialized view");
    refused_with(&written, &SqlState::WRONG_OBJECT_TYPE);

    let emptied = MaterializedView::refresh(n("account_total"))
        .with_no_data()
        .to_string();
    db.batch_execute(&emptied).await?;
    let unreadable = totals(&db)
        .await
        .expect_err("a view a refresh left unpopulated");
    refused_with(&unreadable, &SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE);

    let again = MaterializedView::create(n("account_total"), Query::select().expr(1).take())?
        .if_not_exists()
        .to_string();
    db.batch_execute(&again).await?;
    let kind: String = db
        .query_one(
            "SELECT relkind::text FROM pg_class WHERE relname = 'account_total'",
            &[],
        )
        .await?
        .get(0);
    assert_eq!(kind, "m");

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// `CREATE TABLE AS` makes a table of the query's columns, renamed where
/// names are given, and of its rows unless `WITH NO DATA`; the table is
/// independent of the query from then on.
// [spec:pgorm:req:sql.ddl.create-table-as+1/test]    a table created from a query, with no data
// and then with it, its columns renamed, and too many names refused
#[pgorm_macros::test]
async fn a_table_is_created_from_a_query() -> Result<(), Error> {
    let ctx = TestContext::new("view_create_table_as").await;
    let db = ctx.db.get().await?;
    accounts(&db).await?;

    let empty = Table::create_as(n("active_copy"), active_accounts())?
        .column(n("account_id"))
        .with_no_data()
        .to_string();
    db.batch_execute(&empty).await?;
    let count: i64 = db
        .query_one("SELECT count(*) FROM active_copy", &[])
        .await?
        .get(0);
    assert_eq!(count, 0);
    let columns: Vec<(String, String)> = db
        .query_all(
            "SELECT attname::text, format_type(atttypid, atttypmod) FROM pg_attribute \
             WHERE attrelid = 'active_copy'::regclass AND attnum > 0 ORDER BY attnum",
            &[],
        )
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(
        columns,
        [
            ("account_id".to_owned(), "integer".to_owned()),
            ("name".to_owned(), "text".to_owned()),
            ("active".to_owned(), "boolean".to_owned()),
        ]
    );
    db.batch_execute("INSERT INTO active_copy SELECT id, name, active FROM account WHERE active")
        .await?;
    let copied: Vec<String> = db
        .query_all("SELECT name FROM active_copy ORDER BY account_id", &[])
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(copied, ["ada", "it's"]);

    let full = Table::create_as(n("full_copy"), active_accounts())?.to_string();
    db.batch_execute(&full).await?;
    db.batch_execute("UPDATE account SET name = 'changed'")
        .await?;
    assert_eq!(names(&db, "full_copy").await?, ["ada", "it's"]);
    let kind: String = db
        .query_one(
            "SELECT relkind::text FROM pg_class WHERE relname = 'full_copy'",
            &[],
        )
        .await?
        .get(0);
    assert_eq!(kind, "r");

    let skipped = Table::create_as(n("full_copy"), Query::select().expr(1).take())?
        .if_not_exists()
        .to_string();
    db.batch_execute(&skipped).await?;
    assert_eq!(names(&db, "full_copy").await?, ["ada", "it's"]);

    let too_many = Table::create_as(n("too_many"), Query::select().expr(1).take())?
        .columns([n("a"), n("b")])
        .to_string();
    let refused = db
        .batch_execute(&too_many)
        .await
        .expect_err("more column names than the query has");
    refused_with(&refused, &SqlState::SYNTAX_ERROR);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A body whose tie limit or WINDOW clause PostgreSQL's grammar refuses is
/// refused as the view, recursive view, materialized view or table over it
/// is constructed: built and sent the way a DDL statement is sent, it comes
/// back as pgorm-query's error, and the transaction it would have been sent
/// in is not aborted, so nothing reached the server. Written into the same
/// definitions by hand, the body is refused by the server with the SQLSTATE
/// each check stands for; ordered and declared, it is one the server takes.
// [spec:pgorm:req:sql.ddl.view+1/test]    a view's refused body is refused with nothing sent,
// as the server refuses it, and the corrected body is taken
// [spec:pgorm:req:sql.ddl.materialized-view+1/test]    a materialized view's refused body
// is refused with nothing sent
// [spec:pgorm:req:sql.ddl.create-table-as+1/test]    CREATE TABLE AS's refused body is
// refused with nothing sent
#[pgorm_macros::test]
async fn a_refused_body_is_never_sent() -> Result<(), Error> {
    let ctx = TestContext::new("view_refused_body").await;
    let mut db = ctx.db.get().await?;
    accounts(&db).await?;

    let tied = Query::select()
        .columns([n("id"), n("name")])
        .from(n("account"))
        .limit_with_ties(1)
        .take();
    let windowed = Query::select()
        .column(n("id"))
        .expr_window_name(Func::count(Expr::col(n("id"))), n("w"))
        .from(n("account"))
        .take();
    let redefined = windowed
        .clone()
        .window(n("w"), WindowStatement::new())
        .window(n("w"), WindowStatement::new())
        .take();
    let w = || "w".to_owned();
    let cases = [
        (
            tied.clone(),
            QueryError::TiesWithoutOrder,
            SqlState::SYNTAX_ERROR,
        ),
        (
            windowed.clone(),
            QueryError::WindowUndefined { name: w() },
            SqlState::UNDEFINED_OBJECT,
        ),
        (
            redefined,
            QueryError::WindowRedefined { name: w() },
            SqlState::WINDOWING_ERROR,
        ),
    ];

    db.transaction(async |tx| {
        for (body, refusal, _) in &cases {
            let built = [
                View::create(n("refused"), body.clone()).map(|view| view.to_string()),
                View::create_recursive(n("refused"), n("id"), body.clone())
                    .map(|view| view.to_string()),
                MaterializedView::create(n("refused"), body.clone()).map(|view| view.to_string()),
                Table::create_as(n("refused"), body.clone()).map(|table| table.to_string()),
            ];
            for statement in built {
                let sent = async { tx.batch_execute(&statement?).await }.await;
                match sent {
                    Err(Error::QueryBuilder(error)) => assert_eq!(&error, refusal),
                    other => panic!("expected pgorm-query's refusal, got {other:?}"),
                }
            }
            // A statement the server refused would have aborted the transaction.
            assert_eq!(tx.query_one("SELECT 1", &[]).await?.get::<_, i32>(0), 1);
        }
        Ok::<_, Error>(())
    })
    .await
    .expect("nothing reached the server, so the transaction commits");

    for (body, _, state) in &cases {
        for kind in ["VIEW", "MATERIALIZED VIEW", "TABLE"] {
            let by_hand = format!(r#"CREATE {kind} "refused" AS {body}"#);
            let error = db.batch_execute(&by_hand).await.expect_err(&by_hand);
            refused_with(&error, state);
        }
    }

    // Both active accounts tie for first on `active DESC`.
    let ordered = tied.clone().order_by(n("active"), Order::Desc).take();
    let created = [
        View::create(n("first_active"), ordered.clone())?.to_string(),
        MaterializedView::create(n("first_active_kept"), ordered.clone())?.to_string(),
        Table::create_as(n("first_active_copy"), ordered)?.to_string(),
    ];
    for create in created {
        db.batch_execute(&create).await?;
    }
    for relation in ["first_active", "first_active_kept", "first_active_copy"] {
        assert_eq!(names(&db, relation).await?, ["ada", "it's"]);
    }
    let declared = windowed
        .clone()
        .window(n("w"), WindowStatement::partition_by(n("active")))
        .take();
    db.batch_execute(&View::create(n("counted"), declared)?.to_string())
        .await?;
    let counts: Vec<i64> = db
        .query_all("SELECT count FROM counted ORDER BY id", &[])
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(counts, [2, 2, 1]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A drop reaches its own kind of relation and refuses the other (`42809`),
/// restricts while a view depends on it (`2BP01`) and cascades to it when
/// told, and passes over a missing one under `IF EXISTS`.
// [spec:pgorm:req:sql.ddl.view+1/test]    DROP VIEW restricts, cascades and refuses a
// materialized view
// [spec:pgorm:req:sql.ddl.materialized-view+1/test]    DROP MATERIALIZED VIEW refuses a view
#[pgorm_macros::test]
async fn each_drop_reaches_its_own_kind() -> Result<(), Error> {
    let ctx = TestContext::new("view_drops").await;
    let db = ctx.db.get().await?;
    accounts(&db).await?;

    db.batch_execute(&View::create(n("active_account"), active_accounts())?.to_string())
        .await?;
    let dependent = View::create(
        n("active_name"),
        Query::select()
            .column(n("name"))
            .from(n("active_account"))
            .take(),
    )?
    .to_string();
    db.batch_execute(&dependent).await?;
    db.batch_execute(&MaterializedView::create(n("snapshot"), active_accounts())?.to_string())
        .await?;

    for restricted in [
        View::drop(n("active_account")).to_string(),
        View::drop(n("active_account")).restrict().to_string(),
    ] {
        let refused = db
            .batch_execute(&restricted)
            .await
            .expect_err("a view another view reads");
        refused_with(&refused, &SqlState::DEPENDENT_OBJECTS_STILL_EXIST);
    }
    let wrong_kind = db
        .batch_execute(&View::drop(n("snapshot")).to_string())
        .await
        .expect_err("DROP VIEW on a materialized view");
    refused_with(&wrong_kind, &SqlState::WRONG_OBJECT_TYPE);
    let wrong_kind = db
        .batch_execute(&MaterializedView::drop(n("active_name")).to_string())
        .await
        .expect_err("DROP MATERIALIZED VIEW on a view");
    refused_with(&wrong_kind, &SqlState::WRONG_OBJECT_TYPE);

    db.batch_execute(
        &View::drop(n("active_account"))
            .name(n("missing"))
            .if_exists()
            .cascade()
            .to_string(),
    )
    .await?;
    db.batch_execute(
        &MaterializedView::drop(n("snapshot"))
            .if_exists()
            .to_string(),
    )
    .await?;
    let left: i64 = db
        .query_one(
            "SELECT count(*) FROM pg_class WHERE relname IN ('active_account', 'active_name', 'snapshot')",
            &[],
        )
        .await?
        .get(0);
    assert_eq!(left, 0);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// `ALTER VIEW` is not built, because `Table`'s statements reach what it
/// does: PostgreSQL takes `ALTER TABLE`'s rename, column rename and column
/// default on a view, and its renames on a materialized view.
// [spec:pgorm:req:sql.ddl.view+1/test]    a view and a materialized view are renamed, their
// columns renamed and a view column given an insert default, through Table's statements
#[pgorm_macros::test]
async fn table_alterations_reach_a_view() -> Result<(), Error> {
    let ctx = TestContext::new("view_table_alterations").await;
    let db = ctx.db.get().await?;
    accounts(&db).await?;

    db.batch_execute(&View::create(n("active_account"), active_accounts())?.to_string())
        .await?;
    db.batch_execute(&Table::rename(n("active_account"), n("live_account")).to_string())
        .await?;
    db.batch_execute(&Table::rename_column(n("live_account"), n("name"), n("label")).to_string())
        .await?;
    let defaulted = Table::alter(n("live_account"))
        .modify_column(ColumnDef::new(n("label")).default("anonymous"))
        .to_string();
    db.batch_execute(&defaulted).await?;
    db.batch_execute("INSERT INTO live_account (active) VALUES (true)")
        .await?;
    assert_eq!(
        names(&db, "account").await?,
        ["ada", "it's", "cy", "anonymous"]
    );

    db.batch_execute(&MaterializedView::create(n("snapshot"), active_accounts())?.to_string())
        .await?;
    db.batch_execute(&Table::rename(n("snapshot"), n("kept")).to_string())
        .await?;
    db.batch_execute(&Table::rename_column(n("kept"), n("name"), n("label")).to_string())
        .await?;
    let labels: Vec<String> = db
        .query_all("SELECT label FROM kept ORDER BY id", &[])
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(labels, ["ada", "it's", "anonymous"]);

    drop(db);
    ctx.delete().await;
    Ok(())
}
