#![allow(unused_imports, dead_code)]

//! Primary and unique keys against a live PostgreSQL server.
//!
//! `TableKey` exists so that every key shape the builder can declare is one
//! PostgreSQL accepts; the shapes it refuses — a non-unique kind, an ordered,
//! collated, classed or computed key entry, a predicate, an access method,
//! `NULLS NOT DISTINCT` on a primary key, a second primary key — have no
//! method to reach them, which the `compile_fail` doctests on the type hold.
//! What only a server can settle is the other half: that each shape the
//! builder *can* make is created as the key it names, and behaves as one. Each
//! case below reads the constraint back out of `pg_constraint` and
//! `pg_index`, and sets the row its key refuses beside the row it admits. The
//! one key the builder writes and the server refuses is one naming a column
//! twice, which no type can tell from two columns.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    ColumnDef, ColumnType, Deferrability, Name, RangeType, Table, TableCreateStatement, TableKey,
    Unique,
};
use pgorm::{ConnectionTrait, entity::prelude::*};
use tokio_postgres::error::SqlState;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("table_constraint_tests").await;
    let db = ctx.db.get().await?;

    every_shape_is_created_as_it_names(&db).await?;
    each_key_refuses_what_it_should(&db).await?;
    the_grammar_has_no_other_shape(&db).await?;
    a_later_primary_key_replaces_the_first(&db).await?;
    a_key_naming_a_column_twice_is_refused(&db).await?;
    wide_keys_keep_their_column_order(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

fn n(name: &str) -> Name {
    Name::runtime(name)
}

/// `CREATE TABLE ledger (a int NOT NULL, b int NOT NULL, c int, d text, …)`
/// carrying one key of every shape the builder has: a composite primary key
/// with an `INCLUDE`, a named two-column unique key, a `NULLS NOT DISTINCT`
/// unique key, and a deferrable one.
fn ledger() -> String {
    Table::create(n("ledger"))
        .col(ColumnDef::new(n("a")).integer().not_null())
        .col(ColumnDef::new(n("b")).integer().not_null())
        .col(ColumnDef::new(n("c")).integer())
        .col(ColumnDef::new(n("d")).text())
        .col(ColumnDef::new(n("e")).integer())
        .primary_key(
            TableKey::new(n("a"))
                .col(n("b"))
                .name(n("ledger_pk"))
                .include([n("d")]),
        )
        .unique(TableKey::new(n("c")).col(n("d")).name(n("ledger_c_d")))
        .unique(
            TableKey::new(n("e"))
                .name(n("ledger_e"))
                .nulls_not_distinct(),
        )
        .unique(
            TableKey::new(n("b"))
                .name(n("ledger_b"))
                .deferrability(Deferrability::DeferrableInitiallyDeferred),
        )
        .to_string()
}

/// Each constraint as the catalogue holds it: its type, its key columns and
/// included columns by name, whether nulls are distinct, and whether it is
/// deferrable and initially deferred.
// [spec:pgorm:req:sql.ddl.create-table+16/test]    against a live server: every constraint
// shape the builder makes is created, and is the constraint it names
// [spec:pgorm:req:sql.ddl.deferrability+4/test]
async fn every_shape_is_created_as_it_names(db: &DatabaseConnection) -> Result<(), Error> {
    let create = ledger();
    db.batch_execute(&create).await?;

    let rows = db
        .query_all(
            "SELECT c.conname::text, c.contype::text, \
                    (SELECT string_agg(a.attname, ',' ORDER BY k.ord) \
                       FROM unnest(i.indkey[0:i.indnkeyatts - 1]) WITH ORDINALITY k(attnum, ord) \
                       JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum), \
                    COALESCE((SELECT string_agg(a.attname, ',' ORDER BY k.ord) \
                       FROM unnest(i.indkey[i.indnkeyatts:]) WITH ORDINALITY k(attnum, ord) \
                       JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum), ''), \
                    i.indnullsnotdistinct, c.condeferrable, c.condeferred \
             FROM pg_constraint c JOIN pg_index i ON i.indexrelid = c.conindid \
             WHERE c.conrelid = 'ledger'::regclass ORDER BY c.conname",
            &[],
        )
        .await?;
    let read: Vec<(String, String, String, String, bool, bool, bool)> = rows
        .iter()
        .map(|row| {
            (
                row.get(0),
                row.get(1),
                row.get(2),
                row.get(3),
                row.get(4),
                row.get(5),
                row.get(6),
            )
        })
        .collect();
    let expect = |name: &str, kind: &str, keys: &str, include: &str, nnd, deferrable, deferred| {
        (
            name.to_owned(),
            kind.to_owned(),
            keys.to_owned(),
            include.to_owned(),
            nnd,
            deferrable,
            deferred,
        )
    };
    assert_eq!(
        read,
        [
            expect("ledger_b", "u", "b", "", false, true, true),
            expect("ledger_c_d", "u", "c,d", "", false, false, false),
            expect("ledger_e", "u", "e", "", true, false, false),
            expect("ledger_pk", "p", "a,b", "d", false, false, false),
        ],
        "{create}"
    );

    Ok(())
}

/// Each key refuses the row that repeats it and admits the row its shape
/// says it should: a composite key only a repeat of the whole key, an
/// included column never, a plain unique key any number of nulls, and a
/// `NULLS NOT DISTINCT` one a second null.
// [spec:pgorm:req:sql.ddl.create-table+16/test]    against a live server: each key refuses
// what it should and nothing else
async fn each_key_refuses_what_it_should(db: &DatabaseConnection) -> Result<(), Error> {
    let insert = |a: i32, b: i32, c: Option<i32>, d: &str, e: Option<i32>| {
        let c = c.map_or("NULL".to_owned(), |c| c.to_string());
        let e = e.map_or("NULL".to_owned(), |e| e.to_string());
        format!("INSERT INTO ledger VALUES ({a}, {b}, {c}, '{d}', {e})")
    };

    db.execute(&insert(1, 1, None, "x", None), &[]).await?;
    // Same `a`, another `b`: the composite key is not repeated, and the
    // included `d` is no part of it.
    db.execute(&insert(1, 2, None, "x", Some(1)), &[]).await?;
    // Nulls are distinct under the plain unique key on (c, d).
    db.execute(&insert(2, 3, None, "x", Some(2)), &[]).await?;

    let repeated_key = db.execute(&insert(1, 4, Some(1), "y", Some(3)), &[]).await;
    assert!(repeated_key.is_ok(), "{repeated_key:?}");
    let refused = db
        .execute(&insert(1, 4, Some(2), "z", Some(4)), &[])
        .await
        .expect_err("a repeated primary key");
    refused_with(&refused, &SqlState::UNIQUE_VIOLATION);

    let refused = db
        .execute(&insert(3, 5, Some(1), "y", Some(5)), &[])
        .await
        .expect_err("a repeated (c, d)");
    refused_with(&refused, &SqlState::UNIQUE_VIOLATION);

    let refused = db
        .execute(&insert(3, 6, None, "w", None), &[])
        .await
        .expect_err("a second null under NULLS NOT DISTINCT");
    refused_with(&refused, &SqlState::UNIQUE_VIOLATION);

    Ok(())
}

/// The shapes `TableKey` has no method for are the ones the
/// table-constraint grammar refuses outright (`42601`): a key list with no
/// kind, an ordered, collated, classed or computed key entry, a predicate, an
/// access method, and `NULLS NOT DISTINCT` on a primary key. Each is written
/// raw here, because the builder cannot write it at all; the control beside
/// them is the same table with a key the grammar takes.
// [spec:pgorm:req:sql.ddl.create-table+16/test]    against a live server: the table-constraint
// shapes the builder cannot express are the ones PostgreSQL refuses
async fn the_grammar_has_no_other_shape(db: &DatabaseConnection) -> Result<(), Error> {
    for constraint in [
        "CONSTRAINT r (k)",
        "UNIQUE (k DESC)",
        "UNIQUE (k ASC)",
        "UNIQUE (t text_pattern_ops)",
        r#"UNIQUE (t COLLATE "C")"#,
        "UNIQUE ((lower(t)))",
        "UNIQUE (k) WHERE (k > 0)",
        "UNIQUE USING hash (k)",
        "PRIMARY KEY NULLS NOT DISTINCT (k)",
    ] {
        let create = format!("CREATE TABLE refused (k integer, t text, {constraint})");
        let refused = db.batch_execute(&create).await.expect_err(&create);
        refused_with(&refused, &SqlState::SYNTAX_ERROR);
    }
    db.batch_execute("CREATE TABLE refused (k integer, t text, UNIQUE (k))")
        .await?;

    Ok(())
}

/// The key columns of `table`'s primary key, in key order, and the key's
/// name, as the catalogue holds them.
async fn primary_key_of(db: &DatabaseConnection, table: &str) -> Result<(String, String), Error> {
    let row = db
        .query_one(
            &format!(
                "SELECT c.conname::text, string_agg(a.attname, ',' ORDER BY k.ord) \
                 FROM pg_constraint c, unnest(c.conkey) WITH ORDINALITY k(attnum, ord) \
                 JOIN pg_attribute a ON a.attnum = k.attnum \
                 WHERE c.conrelid = '{table}'::regclass AND c.contype = 'p' \
                 AND a.attrelid = c.conrelid GROUP BY c.conname"
            ),
            &[],
        )
        .await?;
    Ok((row.get(0), row.get(1)))
}

/// A table has one primary key, and a second is refused in every spelling
/// SQL has for one (`42P16`). The builder holds the key in one slot, so a
/// table it builds with two `primary_key` calls has the second's key and
/// only that, under the second's name.
// [spec:pgorm:req:sql.ddl.create-table+16/test]    against a live server: the second key a table
// is given replaces the first, where SQL that declares two is refused
async fn a_later_primary_key_replaces_the_first(db: &DatabaseConnection) -> Result<(), Error> {
    for raw in [
        "CREATE TABLE keyed (a int PRIMARY KEY, b int PRIMARY KEY)",
        "CREATE TABLE keyed (a int PRIMARY KEY, b int, PRIMARY KEY (a, b))",
        "CREATE TABLE keyed (a int, b int, PRIMARY KEY (a), PRIMARY KEY (b))",
    ] {
        let refused = db.batch_execute(raw).await.expect_err(raw);
        refused_with(&refused, &SqlState::INVALID_TABLE_DEFINITION);
    }

    let create = Table::create(n("keyed"))
        .col(ColumnDef::new(n("a")).integer().not_null())
        .col(ColumnDef::new(n("b")).integer().not_null())
        .primary_key(TableKey::new(n("a")).name(n("keyed_first")))
        .primary_key(TableKey::new(n("b")).col(n("a")).name(n("keyed_second")))
        .to_string();
    db.batch_execute(&create).await?;
    assert_eq!(
        primary_key_of(db, "keyed").await?,
        ("keyed_second".to_owned(), "b,a".to_owned()),
        "{create}"
    );

    db.execute("INSERT INTO keyed VALUES (1, 1), (1, 2)", &[])
        .await?;
    let refused = db
        .execute("INSERT INTO keyed VALUES (1, 2)", &[])
        .await
        .expect_err("a repeated (b, a)");
    refused_with(&refused, &SqlState::UNIQUE_VIOLATION);

    Ok(())
}

/// A key names each of its columns once: PostgreSQL refuses one naming a
/// column twice (`42701`), as a primary or a unique key, in `CREATE TABLE` and
/// after `ALTER TABLE`'s `ADD`, its `WITHOUT OVERLAPS` column included. The
/// builder writes such a key as given, the repeat kept, so the refusal is the
/// server's and the key declared is never a narrower one than written. The
/// included columns are no part of the key and may repeat it, which is the
/// control.
// [spec:pgorm:req:sql.ddl.create-table+16/test]    against a live server: a key naming a column
// twice is written as given and refused by the server, whatever spells it
async fn a_key_naming_a_column_twice_is_refused(db: &DatabaseConnection) -> Result<(), Error> {
    let twice = || {
        Table::create(n("twice"))
            .col(ColumnDef::new(n("a")).integer().not_null())
            .col(ColumnDef::new(n("b")).integer().not_null())
            .col(ColumnDef::new_with_type(
                n("p"),
                ColumnType::Range(RangeType::Int4),
            ))
            .to_owned()
    };
    let refused_twice = |create: String, repeat: &'static str| async move {
        assert!(create.contains(repeat), "{create}");
        let refused = db.batch_execute(&create).await.expect_err(&create);
        refused_with(&refused, &SqlState::DUPLICATE_COLUMN);
    };

    refused_twice(
        twice().primary_key((n("a"), n("a"))).to_string(),
        r#"PRIMARY KEY ("a", "a")"#,
    )
    .await;
    refused_twice(
        twice()
            .unique(TableKey::new(n("a")).col(n("b")).col(n("a")))
            .to_string(),
        r#"UNIQUE ("a", "b", "a")"#,
    )
    .await;
    refused_twice(
        twice()
            .unique(TableKey::<Unique>::new(n("p")).without_overlaps(n("p")))
            .to_string(),
        r#"UNIQUE ("p", "p" WITHOUT OVERLAPS)"#,
    )
    .await;

    db.batch_execute(&twice().to_string()).await?;
    refused_twice(
        Table::alter(n("twice"))
            .add_primary_key(TableKey::new(n("b")).cols([n("a"), n("b")]))
            .to_string(),
        r#"ADD PRIMARY KEY ("b", "a", "b")"#,
    )
    .await;
    refused_twice(
        Table::alter(n("twice"))
            .add_unique((n("b"), n("b")))
            .to_string(),
        r#"ADD UNIQUE ("b", "b")"#,
    )
    .await;

    let included = Table::alter(n("twice"))
        .add_unique(
            TableKey::new(n("a"))
                .name(n("twice_a"))
                .include([n("a"), n("b"), n("b")]),
        )
        .to_string();
    db.batch_execute(&included).await?;
    let row = db
        .query_one(
            "SELECT i.indnkeyatts::int, i.indkey::text FROM pg_constraint c \
             JOIN pg_index i ON i.indexrelid = c.conindid WHERE c.conname = 'twice_a'",
            &[],
        )
        .await?;
    let (keyed, columns): (i32, String) = (row.get(0), row.get(1));
    assert_eq!((keyed, columns.as_str()), (1, "1 1 2 2"), "{included}");

    Ok(())
}

/// `c01` to `c<count>`, in order.
fn wide_columns(count: usize) -> Vec<Name> {
    (1..=count).map(column_numbered).collect()
}

/// `c<i>`, two digits wide.
fn column_numbered(i: usize) -> Name {
    n(&format!("c{i:02}"))
}

/// `CREATE TABLE <name> (c01 integer NOT NULL, …, c33 integer NOT NULL)`:
/// one more column than PostgreSQL lets a key or index have.
fn wide(name: &str) -> TableCreateStatement {
    let mut create = Table::create(n(name));
    for column in wide_columns(33) {
        create.col(ColumnDef::new(column).integer().not_null());
    }
    create
}

/// Each key of `table` as the catalogue holds it: its kind and its columns in
/// key order, by name, sorted by the columns.
async fn keys_of(db: &DatabaseConnection, table: &str) -> Result<Vec<(String, String)>, Error> {
    let rows = db
        .query_all(
            &format!(
                "SELECT c.contype::text, string_agg(a.attname, ',' ORDER BY k.ord) AS columns \
                 FROM pg_constraint c, unnest(c.conkey) WITH ORDINALITY k(attnum, ord) \
                 JOIN pg_attribute a ON a.attnum = k.attnum \
                 WHERE c.conrelid = '{table}'::regclass AND c.contype IN ('p', 'u') \
                 AND a.attrelid = c.conrelid GROUP BY c.oid, c.contype ORDER BY columns"
            ),
            &[],
        )
        .await?;
    Ok(rows.iter().map(|row| (row.get(0), row.get(1))).collect())
}

/// `(kind, "c03,c01,c02")` for the columns numbered `order`.
fn key(kind: &str, order: &[usize]) -> (String, String) {
    let columns: Vec<String> = order.iter().map(|i| format!("c{i:02}")).collect();
    (kind.to_owned(), columns.join(","))
}

/// A key written by hand as a tuple keeps the tuple's order, which need not
/// be the table's: a 3-tuple and a 12-tuple, the widest an entity's key value
/// can be, each as a primary and as a unique key, in `CREATE TABLE` and after
/// `ALTER TABLE`'s `ADD`, read back from the catalogue column by column. A
/// key past twelve columns is a computed list, `TableKey::cols`, taken up to
/// PostgreSQL's limit of 32 columns to an index; the 33rd is refused
/// (`54011`).
// [spec:pgorm:req:sql.ddl.create-table+16/test]    against a live server: a 3- and a
// 12-tuple key, and a computed one past twelve, each created in its own column order
// [spec:pgorm:req:sql.ddl.alter-table+14/test]
async fn wide_keys_keep_their_column_order(db: &DatabaseConnection) -> Result<(), Error> {
    let c = column_numbered;
    let create = wide("wide_created")
        .primary_key((c(3), c(1), c(2)))
        .unique((
            c(12),
            c(11),
            c(10),
            c(9),
            c(8),
            c(7),
            c(6),
            c(5),
            c(4),
            c(3),
            c(2),
            c(1),
        ))
        .to_string();
    assert!(
        create.contains(r#"PRIMARY KEY ("c03", "c01", "c02")"#),
        "{create}"
    );
    db.batch_execute(&create).await?;
    assert_eq!(
        keys_of(db, "wide_created").await?,
        [
            key("p", &[3, 1, 2]),
            key("u", &[12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1]),
        ],
        "{create}"
    );

    db.batch_execute(&wide("wide_altered").to_string()).await?;
    let alter = Table::alter(n("wide_altered"))
        .add_primary_key((
            c(1),
            c(12),
            c(2),
            c(11),
            c(3),
            c(10),
            c(4),
            c(9),
            c(5),
            c(8),
            c(6),
            c(7),
        ))
        .add_unique((c(2), c(3), c(1)))
        .add_unique(TableKey::new(c(32)).cols((1..32).map(c)))
        .to_string();
    db.batch_execute(&alter).await?;
    let thirty_two: Vec<usize> = std::iter::once(32).chain(1..32).collect();
    assert_eq!(
        keys_of(db, "wide_altered").await?,
        [
            key("p", &[1, 12, 2, 11, 3, 10, 4, 9, 5, 8, 6, 7]),
            key("u", &[2, 3, 1]),
            key("u", &thirty_two),
        ],
        "{alter}"
    );

    let past_the_limit = Table::alter(n("wide_altered"))
        .add_unique(TableKey::new(c(33)).cols(wide_columns(32)))
        .to_string();
    let refused = db
        .batch_execute(&past_the_limit)
        .await
        .expect_err(&past_the_limit);
    refused_with(&refused, &SqlState::TOO_MANY_COLUMNS);

    Ok(())
}
