//! The live leg: the server must read a hostile name the way libpg_query does.
//!
//! The structural oracle trusts one parser. Here, for one representative site
//! per parse-node kind, real objects are created with the nastiest names and
//! the builder's own statement is run against them: the object has to exist
//! in the catalogue under exactly the name's bytes, the statement has to reach
//! it, and a sentinel table that a stacked `DROP TABLE sentinel` would remove
//! has to survive. Each test owns a throwaway database named after it.

use pgorm::{
    ConnectionTrait, DatabaseConnection, DecodeRaw,
    pgorm_query::{
        Asterisk, ColumnDef, ColumnType, Comment, CommonTableExpression, Domain, DomainCheck, Expr,
        ExtendedWindow, ForeignKey, Func, Index, IntoNamedTable, MergeInsert, MergeUpdate,
        Multirange, Name, OnConflict, Order, Query, Range, Schema, Sequence, SequenceOption, Table,
        TypeName, Values, WindowStatement, WithClause,
        extension::{RangeDefinition, Type},
    },
};

use super::corpus::NUL_NAME;
use crate::common::TestContext;

/// The names the live leg creates objects with: a quote-closing stacked
/// statement aimed at the sentinel, the f48c6425 exfiltration payload, a
/// comment-and-placeholder name, a string-closing name, a right-to-left
/// override, a case-folded name, and dollar quoting.
pub const NASTY: [&str; 7] = [
    "x\"; DROP TABLE sentinel; --",
    "x\\\" , (SELECT 1) AS \"leak",
    "a b/* $1 */",
    "'; DROP TABLE sentinel; --",
    "\u{202E}kcatta",
    "MixedCase",
    "$$tag$$",
];

/// Quote `name` for this file's own fixture SQL, independently of the code
/// under test.
fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Provision the test's database with the sentinel table in it.
pub async fn open(test: &str) -> (TestContext, DatabaseConnection) {
    let ctx = TestContext::new(test).await;
    let db = ctx
        .db
        .get()
        .await
        .expect("a connection to the test database");
    db.batch_execute("CREATE TABLE sentinel (id integer)")
        .await
        .expect("the sentinel table is created");
    (ctx, db)
}

/// Assert the sentinel survived, then drop the database.
pub async fn close(ctx: TestContext, db: DatabaseConnection) {
    let survived = db
        .query_one(
            "SELECT count(*) FROM pg_class WHERE relname = 'sentinel'",
            &[],
        )
        .await
        .map(|row| row.get::<_, i64>(0));
    drop(db);
    ctx.delete().await;
    assert_eq!(
        survived.ok(),
        Some(1),
        "a name executed as SQL: the sentinel table is gone"
    );
}

/// Every message in `err`'s source chain, joined.
pub fn error_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(" <- ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

async fn run(db: &DatabaseConnection, sql: &str) {
    if let Err(err) = db.execute(sql, &[]).await {
        panic!("statement failed: {err}\n    {sql:?}");
    }
}

async fn one_i32(db: &DatabaseConnection, sql: &str) -> i32 {
    match db.query_one(sql, &[]).await {
        Ok(row) => row.get(0),
        Err(err) => panic!("query failed: {err}\n    {sql:?}"),
    }
}

async fn catalogue_count(db: &DatabaseConnection, sql: &str, name: &str) -> i64 {
    match db.query_one(sql, &[&name]).await {
        Ok(row) => row.get(0),
        Err(err) => panic!("catalogue query failed: {err}"),
    }
}

/// `RangeVar.schemaname`, `RangeVar.relname`, `ColumnDef.colname`,
/// `InsertStmt` target columns and `ColumnRef.fields`: a schema, a table and a
/// column all named with the hostile name, created, written and read through
/// the builders.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[tokio::test]
async fn live_relation_schema_and_column_names() {
    let (ctx, db) = open("ident_oracle_live_relation").await;
    for name in NASTY {
        db.batch_execute(&format!("CREATE SCHEMA {}", ident(name)))
            .await
            .expect("the fixture schema is created");
        let n = || Name::runtime(name);
        run(
            &db,
            &Table::create((n(), n()))
                .col(ColumnDef::new(n()).integer())
                .to_string(),
        )
        .await;
        run(
            &db,
            &Query::insert()
                .into_table((n(), n()))
                .columns([n()])
                .values_panic([7.into()])
                .to_string(),
        )
        .await;
        let read = Query::select()
            .column((n(), n()))
            .from((n(), n()))
            .and_where(Expr::col((n(), n())).eq(7))
            .to_string();
        assert_eq!(one_i32(&db, &read).await, 7, "{read:?}");
        let found = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
             JOIN pg_namespace s ON s.oid = c.relnamespace \
             WHERE s.nspname = $1 AND c.relname = $1 AND a.attname = $1",
            name,
        )
        .await;
        assert_eq!(found, 1, "schema, table and column not all named {name:?}");
    }
    close(ctx, db).await;
}

/// `ResTarget.name`, `RangeVar.alias.aliasname`, `RangeSubselect.alias`,
/// `CommonTableExpr.ctename` and `ReturningOption.value`: every alias kind,
/// read back as the server labels it.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[tokio::test]
async fn live_alias_names() {
    let (ctx, db) = open("ident_oracle_live_alias").await;
    db.batch_execute("CREATE TABLE t (c integer); INSERT INTO t VALUES (7)")
        .await
        .expect("the fixture table is created");
    for name in NASTY {
        let n = || Name::runtime(name);
        let projection = Query::select().expr_as(Expr::val(9), n()).to_string();
        let row = db
            .query_one(&projection, &[])
            .await
            .expect("the alias query runs");
        assert_eq!(row.columns()[0].name(), name, "{projection:?}");
        assert_eq!(row.get::<_, i32>(0), 9);

        let table_alias = Query::select()
            .column((n(), Name::runtime("c")))
            .from_as(Name::runtime("t"), n())
            .to_string();
        assert_eq!(one_i32(&db, &table_alias).await, 7, "{table_alias:?}");

        let subquery_alias = Query::select()
            .column((n(), Name::runtime("c")))
            .from_subquery(
                Query::select()
                    .column(Name::runtime("c"))
                    .from(Name::runtime("t"))
                    .take(),
                n(),
            )
            .to_string();
        assert_eq!(one_i32(&db, &subquery_alias).await, 7, "{subquery_alias:?}");

        let cte = CommonTableExpression::new(
            n(),
            Query::select()
                .column(Name::runtime("c"))
                .from(Name::runtime("t"))
                .take(),
        );
        let through_cte = Query::select()
            .column(Asterisk)
            .from(n())
            .with(WithClause::new(cte))
            .to_string();
        assert_eq!(one_i32(&db, &through_cte).await, 7, "{through_cte:?}");

        let c = || Name::runtime("c");
        let touch = || {
            Query::update()
                .table(Name::runtime("t"))
                .value(c(), Expr::col(c()))
                .to_owned()
        };
        let old_renamed = touch()
            .returning(Query::returning().column((n(), c())).old_as(n()))
            .to_string();
        assert_eq!(one_i32(&db, &old_renamed).await, 7, "{old_renamed:?}");
        let new_renamed = touch()
            .returning(Query::returning().column((n(), c())).new_as(n()))
            .to_string();
        assert_eq!(one_i32(&db, &new_renamed).await, 7, "{new_renamed:?}");
    }
    close(ctx, db).await;
}

/// `FuncCall.funcname` and `TypeCast.type_name`: a function and a domain
/// created under the hostile name, called and cast to through the builders.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[tokio::test]
async fn live_function_and_type_names() {
    let (ctx, db) = open("ident_oracle_live_function_type").await;
    for name in NASTY {
        db.batch_execute(&format!(
            "CREATE FUNCTION {}() RETURNS integer LANGUAGE sql AS 'SELECT 7'; \
             CREATE DOMAIN {} AS integer",
            ident(name),
            ident(name)
        ))
        .await
        .expect("the fixture function and domain are created");
        let n = || Name::runtime(name);
        let call = Query::select().expr(Func::named(n())).to_string();
        assert_eq!(one_i32(&db, &call).await, 7, "{call:?}");
        let cast = Query::select().expr(Expr::val(8).cast_as(n())).to_string();
        assert_eq!(one_i32(&db, &cast).await, 8, "{cast:?}");
        let domain = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_type WHERE typtype = 'd' AND typname = $1",
            name,
        )
        .await;
        assert_eq!(domain, 1, "no domain named {name:?}");
    }
    close(ctx, db).await;
}

/// `WindowDef.name` / `WindowDef.refname` / `FuncCall.over`: a window
/// defined, extended and referenced under the hostile name.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[tokio::test]
async fn live_window_names() {
    let (ctx, db) = open("ident_oracle_live_window").await;
    db.batch_execute("CREATE TABLE t (c integer); INSERT INTO t VALUES (7), (7)")
        .await
        .expect("the fixture table is created");
    for name in NASTY {
        let n = || Name::runtime(name);
        let c = || Name::runtime("c");
        let sql = Query::select()
            .expr_window_name(Func::count(Expr::col(c())), n())
            .expr_window_name(Func::count(Expr::col(c())), Name::runtime("ordered"))
            .from(Name::runtime("t"))
            .window(n(), WindowStatement::partition_by(c()))
            .window(
                Name::runtime("ordered"),
                ExtendedWindow::new(n())
                    .order_by(c(), Order::Asc)
                    .to_owned(),
            )
            .limit(1)
            .to_string();
        let row = db
            .query_one(&sql, &[])
            .await
            .unwrap_or_else(|err| panic!("{err}\n    {sql:?}"));
        let counts: (i64, i64) = (row.get(0), row.get(1));
        assert_eq!(counts, (2, 2), "{sql:?}");
    }
    close(ctx, db).await;
}

/// `CollateClause.collname` and `ColumnDef.coll_clause`: a collation created
/// under the hostile name as a copy of `"C"`, named by an expression and by a
/// column definition through the builders.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[tokio::test]
async fn live_collation_names() {
    let (ctx, db) = open("ident_oracle_live_collation").await;
    for (at, name) in NASTY.into_iter().enumerate() {
        db.batch_execute(&format!("CREATE COLLATION {} FROM \"C\"", ident(name)))
            .await
            .expect("the fixture collation is created");
        let n = || Name::runtime(name);
        // Under a copy of "C", every upper-case letter sorts first.
        let compared = Query::select()
            .expr(Expr::case(Expr::val("B").collate(n()).lt(Expr::val("a")), 1).finally(0))
            .to_string();
        assert_eq!(one_i32(&db, &compared).await, 1, "{compared:?}");

        let table = format!("t{at}");
        run(
            &db,
            &Table::create(Name::runtime(table.as_str()))
                .col(ColumnDef::new(Name::runtime("c")).text().collate(n()))
                .to_string(),
        )
        .await;
        let declared = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_attribute a JOIN pg_collation k ON k.oid = a.attcollation \
             WHERE a.attname = 'c' AND k.collname = $1",
            name,
        )
        .await;
        assert_eq!(declared, 1, "no column collated as {name:?}");
    }
    close(ctx, db).await;
}

/// `OnConflictClause.infer.conname`: a unique constraint created under the
/// hostile name and named as the arbiter of an upsert through the builder,
/// which has to reach it for the conflicting row to be updated.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[tokio::test]
async fn live_conflict_constraint_names() {
    let (ctx, db) = open("ident_oracle_live_conflict").await;
    db.batch_execute("CREATE TABLE u (k integer NOT NULL, v integer NOT NULL)")
        .await
        .expect("the fixture table is created");
    for name in NASTY {
        db.batch_execute(&format!(
            "ALTER TABLE u ADD CONSTRAINT {} UNIQUE (k); INSERT INTO u VALUES (1, 1)",
            ident(name)
        ))
        .await
        .expect("the fixture constraint is created");
        let declared = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_constraint WHERE contype = 'u' AND conname = $1",
            name,
        )
        .await;
        assert_eq!(declared, 1, "no unique constraint named {name:?}");

        let upsert = Query::insert()
            .into_table(Name::runtime("u"))
            .columns([Name::runtime("k"), Name::runtime("v")])
            .values_panic([1.into(), 2.into()])
            .on_conflict(
                OnConflict::constraint(Name::runtime(name)).update_column(Name::runtime("v")),
            )
            .to_string();
        run(&db, &upsert).await;
        assert_eq!(
            one_i32(&db, "SELECT v FROM u WHERE k = 1").await,
            2,
            "{upsert:?}"
        );

        db.batch_execute(&format!(
            "ALTER TABLE u DROP CONSTRAINT {}; DELETE FROM u",
            ident(name)
        ))
        .await
        .expect("the fixture constraint is dropped");
    }
    close(ctx, db).await;
}

/// `MergeStmt.relation` — its schema, table and alias — and the `ResTarget`
/// names of an `UPDATE SET` and an `INSERT` column list: a schema, a table
/// and a column created under the hostile name, then merged into through the
/// builder with the table aliased by the name too. The matched row's column
/// has to be updated and the unmatched source row inserted under it.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[tokio::test]
async fn live_merge_names() {
    let (ctx, db) = open("ident_oracle_live_merge").await;
    db.batch_execute(
        "CREATE TABLE src (k integer, v integer); INSERT INTO src VALUES (1, 10), (2, 20)",
    )
    .await
    .expect("the fixture source is created");
    let src = || Name::runtime("src");
    let k = || Name::runtime("k");
    for name in NASTY {
        db.batch_execute(&format!(
            "CREATE SCHEMA {i}; CREATE TABLE {i}.{i} (k integer PRIMARY KEY, {i} integer); \
             INSERT INTO {i}.{i} VALUES (1, 1)",
            i = ident(name)
        ))
        .await
        .expect("the fixture table is created");
        let declared = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_class c JOIN pg_namespace s ON s.oid = c.relnamespace \
             JOIN pg_attribute a ON a.attrelid = c.oid \
             WHERE s.nspname = $1 AND c.relname = $1 AND a.attname = $1",
            name,
        )
        .await;
        assert_eq!(declared, 1, "no table {name:?}.{name:?} with that column");

        let n = || Name::runtime(name);
        let merge = Query::merge(
            (n(), n()).into_named_table().alias(n()),
            src(),
            Expr::col((n(), k())).equals((src(), k())),
        )
        .when_matched(MergeUpdate::value(
            n(),
            Expr::col((src(), Name::runtime("v"))),
        ))
        .when_not_matched(
            MergeInsert::value(k(), Expr::col((src(), k())))
                .and_value(n(), Expr::col((src(), Name::runtime("v")))),
        )
        .to_string();
        run(&db, &merge).await;

        let read = Query::select()
            .column(n())
            .from((n(), n()))
            .order_by(k(), Order::Asc)
            .to_string();
        let written: Vec<i32> = match db.query_all(&read, &[]).await {
            Ok(rows) => rows.iter().map(|row| row.get(0)).collect(),
            Err(err) => panic!("query failed: {err}\n    {read:?}"),
        };
        assert_eq!(written, [10, 20], "{merge:?}");
    }
    close(ctx, db).await;
}

/// `CreateSeqStmt.sequence`, the `OWNED BY` column list, `AlterSeqStmt`,
/// `RenameStmt` and `DropStmt` over a sequence: a schema, a table and a column
/// created under the hostile name, and a sequence in that schema owned by the
/// column, then restarted, renamed and dropped through the builders. A table
/// and a sequence share one namespace, so the sequence's names are the hostile
/// name with a suffix, every hostile byte still in them.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[tokio::test]
async fn live_sequence_names() {
    let (ctx, db) = open("ident_oracle_live_sequence").await;
    for name in NASTY {
        let n = || Name::runtime(name);
        db.batch_execute(&format!(
            "CREATE SCHEMA {schema}; CREATE TABLE {schema}.{schema} ({schema} bigint)",
            schema = ident(name)
        ))
        .await
        .expect("the fixture schema, table and column are created");
        let sequence = format!("{name}_seq");
        let renamed = format!("{name}_renamed");
        let in_schema = |relname: &str| (n(), Name::runtime(relname));

        run(
            &db,
            &Sequence::create(in_schema(&sequence))
                .options(SequenceOption::StartWith(5))
                .owned_by((n(), n()), n())
                .to_string(),
        )
        .await;
        let owned = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_class s JOIN pg_namespace ns ON ns.oid = s.relnamespace \
             JOIN pg_depend d ON d.objid = s.oid AND d.deptype = 'a' \
             JOIN pg_attribute a ON a.attrelid = d.refobjid AND a.attnum = d.refobjsubid \
             JOIN pg_class t ON t.oid = a.attrelid \
             WHERE s.relkind = 'S' AND s.relname = $1 || '_seq' AND ns.nspname = $1 \
             AND t.relname = $1 AND a.attname = $1",
            name,
        )
        .await;
        assert_eq!(owned, 1, "no sequence {sequence:?} owned by {name:?}");

        run(
            &db,
            &Sequence::alter(in_schema(&sequence))
                .restart_with(42)
                .to_string(),
        )
        .await;
        let next = catalogue_count(
            &db,
            "SELECT nextval(s.oid) FROM pg_class s JOIN pg_namespace ns ON ns.oid = s.relnamespace \
             WHERE s.relkind = 'S' AND s.relname = $1 || '_seq' AND ns.nspname = $1",
            name,
        )
        .await;
        assert_eq!(next, 42, "the restart did not reach {sequence:?}");

        run(
            &db,
            &Sequence::rename(in_schema(&sequence), Name::runtime(renamed.as_str())).to_string(),
        )
        .await;
        let moved = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_class WHERE relkind = 'S' AND relname = $1",
            &renamed,
        )
        .await;
        assert_eq!(moved, 1, "no sequence renamed to {renamed:?}");

        run(&db, &Sequence::drop(in_schema(&renamed)).to_string()).await;
        let left = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_class WHERE relkind = 'S' AND relname = $1",
            &renamed,
        )
        .await;
        assert_eq!(left, 0, "the sequence {renamed:?} survived its drop");
    }
    close(ctx, db).await;
}

/// `CreateSchemaStmt.schemaname`, `RenameStmt.subname` / `newname` and a
/// schema `DropStmt`'s names: a schema created under the hostile name, a
/// table put in it, the schema renamed, then dropped with what it holds.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[tokio::test]
async fn live_schema_names() {
    let (ctx, db) = open("ident_oracle_live_schema").await;
    for name in NASTY {
        let renamed = format!("{name}_renamed");
        run(&db, &Schema::create(Name::runtime(name)).to_string()).await;
        let created = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_namespace WHERE nspname = $1",
            name,
        )
        .await;
        assert_eq!(created, 1, "no schema {name:?}");
        db.batch_execute(&format!("CREATE TABLE {}.held (id integer)", ident(name)))
            .await
            .expect("a table is created in the schema");

        run(
            &db,
            &Schema::rename(Name::runtime(name), Name::runtime(renamed.as_str())).to_string(),
        )
        .await;
        let moved = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_class c JOIN pg_namespace ns ON ns.oid = c.relnamespace \
             WHERE ns.nspname = $1 AND c.relname = 'held'",
            &renamed,
        )
        .await;
        assert_eq!(moved, 1, "the table did not move to {renamed:?}");

        run(
            &db,
            &Schema::drop(Name::runtime(renamed.as_str()))
                .cascade()
                .to_string(),
        )
        .await;
        let left = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_namespace WHERE nspname = $1",
            &renamed,
        )
        .await;
        assert_eq!(left, 0, "the schema {renamed:?} survived its drop");
    }
    close(ctx, db).await;
}

/// `CreateDomainStmt.domainname`, a domain check's `Constraint.conname`, a
/// column typed with the domain, `RenameStmt.subname` / `newname` and a
/// domain `DropStmt`'s names: a domain created under the hostile name with a
/// check of that name, a table column typed with it, both renamed, then the
/// domain dropped with the column.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[tokio::test]
async fn live_domain_names() {
    let (ctx, db) = open("ident_oracle_live_domain").await;
    for name in NASTY {
        let renamed = format!("{name}_renamed");
        run(
            &db,
            &Domain::create(Name::runtime(name), ColumnType::Integer)
                .check(DomainCheck::new(Expr::domain_value().gt(0)).name(Name::runtime(name)))
                .to_string(),
        )
        .await;
        let created = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_type t JOIN pg_constraint c ON c.contypid = t.oid \
             WHERE t.typname = $1 AND t.typtype = 'd' AND c.conname = t.typname",
            name,
        )
        .await;
        assert_eq!(created, 1, "no domain {name:?} with its check");
        run(
            &db,
            &Table::create(Name::runtime("typed"))
                .col(ColumnDef::new_with_type(
                    Name::runtime("v"),
                    ColumnType::Domain {
                        name: Name::runtime(name),
                        schema: None,
                        base: std::sync::Arc::new(ColumnType::Integer),
                    },
                ))
                .to_string(),
        )
        .await;

        run(
            &db,
            &Domain::alter(Name::runtime(name))
                .rename_constraint(Name::runtime(name), Name::runtime(renamed.as_str()))
                .to_string(),
        )
        .await;
        run(
            &db,
            &Domain::alter(Name::runtime(name))
                .rename_to(Name::runtime(renamed.as_str()))
                .to_string(),
        )
        .await;
        let moved = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_attribute a JOIN pg_type t ON t.oid = a.atttypid \
             JOIN pg_constraint c ON c.contypid = t.oid \
             WHERE a.attrelid = 'typed'::regclass AND t.typname = $1 AND c.conname = t.typname",
            &renamed,
        )
        .await;
        assert_eq!(moved, 1, "the column is not typed with {renamed:?}");

        run(
            &db,
            &Domain::drop(Name::runtime(renamed.as_str()))
                .cascade()
                .to_string(),
        )
        .await;
        let left = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_type WHERE typname = $1",
            &renamed,
        )
        .await;
        assert_eq!(left, 0, "the domain {renamed:?} survived its drop");
        db.batch_execute("DROP TABLE typed")
            .await
            .expect("the table is dropped");
    }
    close(ctx, db).await;
}

/// `CompositeTypeStmt.typevar`, and its attributes' `ColumnDef.colname` and
/// `coll_clause`: a collation created under the hostile name, then a
/// composite type of that name whose one attribute has the name too and is
/// collated by it, checked in the catalogue and dropped through the builders.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[tokio::test]
async fn live_composite_type_names() {
    let (ctx, db) = open("ident_oracle_live_composite").await;
    for name in NASTY {
        db.batch_execute(&format!("CREATE COLLATION {} FROM \"C\"", ident(name)))
            .await
            .expect("the fixture collation is created");
        let n = || Name::runtime(name);
        run(
            &db,
            &Type::create(n())
                .attribute_collated(n(), ColumnType::Text, n())
                .to_string(),
        )
        .await;
        let declared = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_type t JOIN pg_attribute a ON a.attrelid = t.typrelid \
             JOIN pg_collation k ON k.oid = a.attcollation \
             WHERE t.typtype = 'c' AND t.typname = $1 AND a.attname = $1 AND k.collname = $1",
            name,
        )
        .await;
        assert_eq!(declared, 1, "no composite {name:?} with that attribute");

        run(&db, &Type::drop(n()).to_string()).await;
        let left =
            catalogue_count(&db, "SELECT count(*) FROM pg_type WHERE typname = $1", name).await;
        assert_eq!(left, 0, "the composite {name:?} survived its drop");
    }
    close(ctx, db).await;
}

/// `CreateRangeStmt.type_name` and its options' `TypeName.names`: a schema,
/// a btree operator class over `float8`, a difference function and a
/// collation created under the hostile name, then a range type of that name
/// in that schema ordered and measured by them, with a multirange named after
/// it, and a second range over `text` collated by it — each checked in the
/// catalogue and dropped through the builders. A value cast to the range type,
/// and one cast to its multirange, by `Expr::as_range` reaches that type,
/// inline and bound.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[tokio::test]
async fn live_range_type_names() {
    let (ctx, db) = open("ident_oracle_live_range").await;
    for name in NASTY {
        db.batch_execute(&format!(
            "CREATE SCHEMA {i}; CREATE COLLATION {i} FROM \"C\"; \
             CREATE FUNCTION {i}(float8, float8) RETURNS float8 \
             LANGUAGE sql IMMUTABLE AS 'SELECT $1 - $2'; \
             CREATE OPERATOR CLASS {i} FOR TYPE float8 USING btree AS \
             OPERATOR 1 <, OPERATOR 2 <=, OPERATOR 3 =, OPERATOR 4 >=, OPERATOR 5 >, \
             FUNCTION 1 btfloat8cmp(float8, float8)",
            i = ident(name)
        ))
        .await
        .expect("the fixture schema, collation, function and operator class are created");
        let n = || Name::runtime(name);
        let multirange = format!("{name}_m");
        let text_range = format!("{name}_t");

        run(
            &db,
            &Type::create((n(), n()))
                .as_range(
                    RangeDefinition::new(ColumnType::Double)
                        .subtype_opclass(n())
                        .subtype_diff(n())
                        .multirange_type_name((n(), Name::runtime(multirange.as_str()))),
                )
                .to_string(),
        )
        .await;
        let declared = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_range r JOIN pg_type t ON t.oid = r.rngtypid \
             JOIN pg_namespace tn ON tn.oid = t.typnamespace \
             JOIN pg_opclass o ON o.oid = r.rngsubopc \
             JOIN pg_proc p ON p.oid = r.rngsubdiff \
             JOIN pg_type m ON m.oid = r.rngmultitypid \
             JOIN pg_namespace mn ON mn.oid = m.typnamespace \
             WHERE t.typname = $1 AND tn.nspname = $1 AND o.opcname = $1 \
             AND p.proname = $1 AND m.typname = $1 || '_m' AND mn.nspname = $1",
            name,
        )
        .await;
        assert_eq!(declared, 1, "no range {name:?} with those options");

        let cast = Query::select()
            .expr(
                Expr::val(Range::from(1.5..2.5).to_string())
                    .as_range(TypeName::new(n()).schema(n())),
            )
            .to_owned();
        let multirange_cast = Query::select()
            .expr(
                Expr::val(Multirange::from(vec![Range::from(1.5..2.5)]).to_string())
                    .as_range(TypeName::new(Name::runtime(multirange.as_str())).schema(n())),
            )
            .to_owned();
        for (cast, wanted, text) in [
            (&cast, name.to_owned(), "[1.5,2.5)"),
            (&multirange_cast, multirange.clone(), "{[1.5,2.5)}"),
        ] {
            for (sql, values) in [(cast.to_string(), Values(Vec::new())), cast.build()] {
                let typed: (String, String, String) = (
                    format!(
                        "SELECT s.nspname::text, t.typname::text, q.r::text FROM ({sql}) AS q(r) \
                     JOIN pg_type t ON t.oid = pg_typeof(q.r) \
                     JOIN pg_namespace s ON s.oid = t.typnamespace"
                    ),
                    values,
                )
                    .into_tuple()
                    .one(&db)
                    .await
                    .unwrap_or_else(|err| panic!("a value cast to range {name:?}: {err}"));
                assert_eq!(
                    typed,
                    (name.to_owned(), wanted.clone(), text.to_owned()),
                    "{sql}"
                );
            }
        }

        run(
            &db,
            &Type::create(Name::runtime(text_range.as_str()))
                .as_range(RangeDefinition::new(ColumnType::Text).collation(n()))
                .to_string(),
        )
        .await;
        let collated = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_range r JOIN pg_type t ON t.oid = r.rngtypid \
             JOIN pg_collation k ON k.oid = r.rngcollation \
             WHERE t.typname = $1 || '_t' AND k.collname = $1",
            name,
        )
        .await;
        assert_eq!(collated, 1, "no range {text_range:?} collated by {name:?}");

        run(&db, &Type::drop((n(), n())).to_string()).await;
        run(
            &db,
            &Type::drop(Name::runtime(text_range.as_str())).to_string(),
        )
        .await;
        let left = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_type \
             WHERE typname::text IN ($1::text, $1::text || '_m', $1::text || '_t')",
            name,
        )
        .await;
        assert_eq!(left, 0, "a range type of {name:?} survived its drop");
    }
    close(ctx, db).await;
}

/// `IndexStmt.idxname`, `Constraint.conname`, `CreateEnumStmt` type names and
/// labels, and `COMMENT ON` targets: DDL-only names, checked in the catalogue.
// [spec:pgorm:req:security.ident-oracle+17/test]
#[tokio::test]
async fn live_ddl_object_names_and_labels() {
    let (ctx, db) = open("ident_oracle_live_ddl").await;
    db.batch_execute("CREATE TABLE t (c integer PRIMARY KEY); CREATE TABLE r (rc integer UNIQUE)")
        .await
        .expect("the fixture tables are created");
    for name in NASTY {
        let n = || Name::runtime(name);
        run(
            &db,
            &Index::create(Name::runtime("t"), Name::runtime("c"))
                .name(n())
                .to_string(),
        )
        .await;
        let index = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_class WHERE relkind = 'i' AND relname = $1",
            name,
        )
        .await;
        assert_eq!(index, 1, "no index named {name:?}");

        run(
            &db,
            &ForeignKey::create(
                Name::runtime("t"),
                Name::runtime("c"),
                Name::runtime("r"),
                Name::runtime("rc"),
            )
            .name(n())
            .to_string(),
        )
        .await;
        let constraint = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_constraint WHERE contype = 'f' AND conname = $1",
            name,
        )
        .await;
        assert_eq!(constraint, 1, "no foreign key named {name:?}");

        run(&db, &Type::create(n()).as_enum().values([name]).to_string()).await;
        let label = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_enum e JOIN pg_type t ON t.oid = e.enumtypid \
             WHERE t.typname = $1 AND e.enumlabel = $1",
            name,
        )
        .await;
        assert_eq!(label, 1, "no enum type {name:?} with that label");

        run(
            &db,
            &Comment::on_table(Name::runtime("t"), name).to_string(),
        )
        .await;
        run(
            &db,
            &Comment::on_column(Name::runtime("t"), Name::runtime("c"), name).to_string(),
        )
        .await;
        let described = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_description d JOIN pg_class c ON c.oid = d.objoid \
             WHERE c.relname = 't' AND d.description = $1",
            name,
        )
        .await;
        assert_eq!(described, 2, "comment text did not round-trip as {name:?}");

        run(
            &db,
            &Table::rename_constraint(Name::runtime("t"), n(), Name::runtime("renamed"))
                .to_string(),
        )
        .await;
        run(
            &db,
            &Table::rename_constraint(Name::runtime("t"), Name::runtime("renamed"), n())
                .to_string(),
        )
        .await;
        let renamed = catalogue_count(
            &db,
            "SELECT count(*) FROM pg_constraint WHERE contype = 'f' AND conname = $1",
            name,
        )
        .await;
        assert_eq!(renamed, 1, "no foreign key renamed back to {name:?}");
        run(
            &db,
            &Table::alter(Name::runtime("t"))
                .drop_constraint(n())
                .to_string(),
        )
        .await;
        run(&db, &Index::drop(n()).to_string()).await;
        run(&db, &Type::drop(n()).to_string()).await;
    }
    close(ctx, db).await;
}

/// NUL, live, at one representative site: `Table::create` renders the name
/// with its NUL byte, and sending the statement fails in the client's
/// protocol encoder — no statement reaches the server, the connection stays
/// usable, and no table exists afterwards.
// [spec:pgorm:req:security.ident-oracle.nul+2/test]
#[tokio::test]
async fn live_nul_name_never_reaches_the_server() {
    let (ctx, db) = open("ident_oracle_live_nul").await;
    let sql = Table::create(Name::runtime(NUL_NAME))
        .col(ColumnDef::new(Name::runtime("c")).integer())
        .to_string();
    assert!(sql.contains('\0'), "the NUL byte was not rendered: {sql:?}");
    let prepared = db.execute(&sql, &[]).await;
    let simple = db.batch_execute(&sql).await;
    for (route, outcome) in [("extended", prepared.err()), ("simple", simple.err())] {
        let err = outcome.unwrap_or_else(|| panic!("the {route} protocol sent a NUL-bearing name"));
        let chain = error_chain(&err);
        assert!(
            chain.contains("error encoding message to server")
                && chain.contains("string contains embedded null"),
            "the {route} refusal is not the client-side protocol encoder's: {chain}"
        );
    }
    let alive = one_i32(&db, "SELECT 1").await;
    assert_eq!(alive, 1, "the connection did not survive the refusal");
    let tables = catalogue_count(
        &db,
        "SELECT count(*) FROM pg_class WHERE relname LIKE $1",
        "nul%",
    )
    .await;
    assert_eq!(tables, 0, "a NUL-bearing CREATE TABLE reached the server");
    close(ctx, db).await;
}
