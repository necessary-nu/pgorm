#![allow(unused_imports, dead_code)]

use pgorm_query::ExprTrait;
pub mod common;

pub use common::{TestContext, setup::*};

use std::{
    num::NonZeroU32,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use futures::{SinkExt, StreamExt, TryStreamExt, stream};
use pgorm::{
    Bytes, ConnectionTrait, CopyTrait, DatabaseConnection, Error, TransactionTrait,
    metric::{InstrumentedPool, MetricsCollector, QueryContext},
    pgorm_query::{
        CopyBinary, CopyCsv, CopyFormat, CopyFromStatement, CopyIgnore, CopyLogVerbosity,
        CopySource, CopyText, CopyToStatement, Expr, Func, MergeUpdate, Name, Order, Query,
        error::Error as QueryError,
    },
};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

fn n(name: &str) -> Name {
    Name::runtime(name)
}

const ITEMS: &str = "CREATE TABLE item (
    id int PRIMARY KEY,
    name text,
    qty int NOT NULL DEFAULT 7
)";

async fn items(
    db: &DatabaseConnection,
    table: &str,
) -> Result<Vec<(i32, Option<String>, i32)>, Error> {
    let rows = db
        .query_all(
            &format!("SELECT id, name, qty FROM {table} ORDER BY id"),
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect())
}

/// Write `data` through a fresh COPY FROM STDIN and finish it.
async fn copy_in_all<C, F>(
    db: &mut C,
    copy: &CopyFromStatement<F>,
    data: &[&str],
) -> Result<u64, Error>
where
    C: CopyTrait + Send,
    F: CopyFormat,
{
    let mut sink = db.copy_in(copy).await?;
    for chunk in data {
        sink.send(Bytes::copy_from_slice(chunk.as_bytes())).await?;
    }
    sink.finish().await
}

/// Every chunk of a COPY TO STDOUT, joined.
async fn copy_out_all<C, F>(db: &mut C, copy: &CopyToStatement<F>) -> Result<Vec<u8>, Error>
where
    C: CopyTrait + Send,
    F: CopyFormat,
{
    let chunks: Vec<Bytes> = db.copy_out(copy).await?.try_collect().await?;
    Ok(chunks.concat())
}

fn sqlstate(error: &Error) -> Option<&SqlState> {
    match error {
        Error::Postgres(error) => error.code(),
        _ => None,
    }
}

// [spec:pgorm:req:conn.copy+2/test]    text format both ways: rows in, the same text out, NULL as \N
#[pgorm_macros::test]
async fn copy_text_round_trip() -> Result<(), Error> {
    let ctx = TestContext::new("copy_text_round_trip").await;
    let mut db = ctx.db.get().await?;
    db.execute(ITEMS, &[]).await?;

    let rows = copy_in_all(
        &mut db,
        &CopyFromStatement::new(n("item"), CopyText),
        // A chunk boundary need not fall on a row boundary.
        &["1\tapple\t3\n2\tp", "ear\t4\n3\t\\N\t5\n"],
    )
    .await?;
    assert_eq!(rows, 3);
    assert_eq!(
        items(&db, "item").await?,
        [
            (1, Some("apple".to_owned()), 3),
            (2, Some("pear".to_owned()), 4),
            (3, None, 5),
        ]
    );

    let out = copy_out_all(&mut db, &CopyToStatement::new(n("item"), CopyText)).await?;
    assert_eq!(
        String::from_utf8(out).expect("text COPY is UTF-8 here"),
        "1\tapple\t3\n2\tpear\t4\n3\t\\N\t5\n"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.copy+2/test]    csv both ways with a header, quoting and a forced quote
#[pgorm_macros::test]
async fn copy_csv_round_trip() -> Result<(), Error> {
    let ctx = TestContext::new("copy_csv_round_trip").await;
    let mut db = ctx.db.get().await?;
    db.execute(ITEMS, &[]).await?;

    let copy = CopyFromStatement::new(n("item"), CopyCsv)
        .header_match()
        .null_string("NULL")
        .to_owned();
    let rows = copy_in_all(
        &mut db,
        &copy,
        &[
            "id,name,qty\n",
            "1,\"a, \"\"quoted\"\" name\",3\n",
            "2,NULL,4\n",
        ],
    )
    .await?;
    assert_eq!(rows, 2);
    assert_eq!(
        items(&db, "item").await?,
        [(1, Some("a, \"quoted\" name".to_owned()), 3), (2, None, 4)]
    );

    let out = copy_out_all(
        &mut db,
        &CopyToStatement::new(n("item"), CopyCsv)
            .header()
            .delimiter(';')
            .force_quote([n("qty")])
            .to_owned(),
    )
    .await?;
    assert_eq!(
        String::from_utf8(out).expect("csv COPY is UTF-8 here"),
        "id;name;qty\n1;\"a, \"\"quoted\"\" name\";\"3\"\n2;;\"4\"\n"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.copy+2/test]    binary out of one table is binary in to another, value for value
#[pgorm_macros::test]
async fn copy_binary_round_trip() -> Result<(), Error> {
    let ctx = TestContext::new("copy_binary_round_trip").await;
    let mut db = ctx.db.get().await?;
    db.execute(ITEMS, &[]).await?;
    db.execute("CREATE TABLE item_copy (LIKE item INCLUDING ALL)", &[])
        .await?;
    db.execute(
        "INSERT INTO item VALUES (1, 'apple', 3), (2, NULL, 4), (3, 'ü ☃', 5)",
        &[],
    )
    .await?;

    let out = copy_out_all(&mut db, &CopyToStatement::new(n("item"), CopyBinary)).await?;
    assert!(out.starts_with(b"PGCOPY\n\xff\r\n\0"), "binary signature");

    let mut sink = db
        .copy_in(&CopyFromStatement::new(n("item_copy"), CopyBinary))
        .await?;
    // Split the binary stream at an arbitrary point, as a reader of a file
    // would deliver it.
    let (head, tail) = out.split_at(13);
    sink.send(Bytes::copy_from_slice(head)).await?;
    sink.send(Bytes::copy_from_slice(tail)).await?;
    assert_eq!(sink.finish().await?, 3);

    assert_eq!(items(&db, "item_copy").await?, items(&db, "item").await?);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.copy+2/test]    a column subset leaves the other columns to their defaults
#[pgorm_macros::test]
async fn copy_column_subset_takes_defaults() -> Result<(), Error> {
    let ctx = TestContext::new("copy_column_subset_defaults").await;
    let mut db = ctx.db.get().await?;
    db.execute(ITEMS, &[]).await?;

    let copy = CopyFromStatement::new(n("item"), CopyCsv)
        .columns([n("name"), n("id")])
        .to_owned();
    assert_eq!(
        copy_in_all(&mut db, &copy, &["fig,10\nkiwi,11\n"]).await?,
        2
    );
    assert_eq!(
        items(&db, "item").await?,
        [
            (10, Some("fig".to_owned()), 7),
            (11, Some("kiwi".to_owned()), 7),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.copy+2/test]    a unique violation mid-copy fails finish with 23505 and aborts the transaction
#[pgorm_macros::test]
async fn copy_violation_aborts_the_transaction() -> Result<(), Error> {
    let ctx = TestContext::new("copy_violation_aborts_tx").await;
    let mut db = ctx.db.get().await?;
    db.execute(ITEMS, &[]).await?;

    let outcome = db
        .transaction(async |txn| {
            txn.execute("INSERT INTO item (id) VALUES (100)", &[])
                .await?;
            let error = copy_in_all(
                txn,
                &CopyFromStatement::new(n("item"), CopyCsv),
                &["1,a,1\n2,b,2\n1,again,3\n3,c,3\n"],
            )
            .await
            .expect_err("the duplicate key is refused");
            assert_eq!(
                sqlstate(&error),
                Some(&SqlState::UNIQUE_VIOLATION),
                "{error}"
            );

            let after = txn
                .execute("SELECT 1", &[])
                .await
                .expect_err("a failed COPY aborts the transaction it ran in");
            assert_eq!(
                sqlstate(&after),
                Some(&SqlState::IN_FAILED_SQL_TRANSACTION),
                "{after}"
            );
            roll_back::<()>()
        })
        .await;
    assert_rolled_back(outcome);

    assert_eq!(items(&db, "item").await?, []);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.copy+2/test]    a dropped sink aborts the COPY: no rows, and the connection keeps working
#[pgorm_macros::test]
async fn copy_dropped_sink_writes_nothing() -> Result<(), Error> {
    let ctx = TestContext::new("copy_dropped_sink_nothing").await;
    let mut db = ctx.db.get().await?;
    db.execute(ITEMS, &[]).await?;

    {
        let mut sink = db
            .copy_in(&CopyFromStatement::new(n("item"), CopyText))
            .await?;
        sink.send(Bytes::from_static(b"1\tapple\t3\n2\tpear\t4\n"))
            .await?;
        sink.flush().await?;
    }

    assert_eq!(items(&db, "item").await?, []);
    assert_eq!(
        db.execute("INSERT INTO item (id) VALUES (1)", &[]).await?,
        1
    );

    // The same inside a transaction leaves the transaction aborted.
    let outcome = db
        .transaction(async |txn| {
            {
                let mut sink = txn
                    .copy_in(&CopyFromStatement::new(n("item"), CopyText))
                    .await?;
                sink.send(Bytes::from_static(b"5\tplum\t3\n")).await?;
            }
            let after = txn.execute("SELECT 1", &[]).await.expect_err("aborted");
            assert_eq!(sqlstate(&after), Some(&SqlState::IN_FAILED_SQL_TRANSACTION));
            roll_back::<()>()
        })
        .await;
    assert_rolled_back(outcome);
    assert_eq!(items(&db, "item").await?, [(1, None, 7)]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// One generated row of the large-stream test, 100 bytes wide.
fn wide_row(id: u32) -> String {
    format!("{id:08}\t{}\n", "x".repeat(90))
}

// [spec:pgorm:req:conn.copy+2/test]    several megabytes stream through both directions chunk by chunk
#[pgorm_macros::test]
async fn copy_streams_several_megabytes() -> Result<(), Error> {
    let ctx = TestContext::new("copy_streams_megabytes").await;
    let mut db = ctx.db.get().await?;
    db.execute("CREATE TABLE wide (id int PRIMARY KEY, body text)", &[])
        .await?;

    const ROWS: u32 = 100_000;
    let generated = Arc::new(Mutex::new(0_u32));
    let counter = generated.clone();
    // A lazy source: each 1000-row chunk is built only when the sink asks
    // for the next one.
    let mut source = stream::iter(0..ROWS / 1000).map(move |block| {
        *counter.lock().unwrap() += 1;
        let chunk: String = (block * 1000..(block + 1) * 1000).map(wide_row).collect();
        Ok::<_, Error>(Bytes::from(chunk))
    });

    let mut sink = db
        .copy_in(&CopyFromStatement::new(n("wide"), CopyText))
        .await?;
    sink.send_all(&mut source).await?;
    assert_eq!(sink.finish().await?, u64::from(ROWS));
    assert_eq!(*generated.lock().unwrap(), ROWS / 1000);

    let mut out = db
        .copy_out(&CopyToStatement::new(
            Query::select()
                .columns([n("id"), n("body")])
                .from(n("wide"))
                .order_by(n("id"), Order::Asc)
                .take(),
            CopyText,
        ))
        .await?;
    let (mut chunks, mut bytes) = (0_usize, 0_usize);
    let mut first = None;
    while let Some(chunk) = out.next().await {
        let chunk = chunk?;
        first.get_or_insert_with(|| chunk.clone());
        chunks += 1;
        bytes += chunk.len();
    }
    drop(out);
    // The id column comes back unpadded.
    let expected: usize = (0..ROWS).map(|id| id.to_string().len() + 92).sum();
    assert_eq!(bytes, expected, "every byte arrives");
    assert!(
        chunks >= ROWS as usize,
        "the server sends a chunk per row: {chunks}"
    );
    assert_eq!(
        first.as_deref(),
        Some(format!("0\t{}\n", "x".repeat(90)).as_bytes())
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.copy+2/test]    one chunk longer than a message carries is split, not refused
#[pgorm_macros::test]
async fn copy_in_one_oversized_chunk() -> Result<(), Error> {
    let ctx = TestContext::new("copy_in_one_oversized_chunk").await;
    let mut db = ctx.db.get().await?;
    db.execute("CREATE TABLE wide (id int PRIMARY KEY, body text)", &[])
        .await?;

    let rows = 40_000_u32;
    let chunk: String = (0..rows).map(wide_row).collect();
    assert!(chunk.len() > 3 << 20, "longer than three messages");
    let mut sink = db
        .copy_in(&CopyFromStatement::new(n("wide"), CopyText))
        .await?;
    sink.send(Bytes::from(chunk)).await?;
    assert_eq!(sink.finish().await?, u64::from(rows));

    let count: i64 = db
        .query_one("SELECT count(*) FROM wide WHERE length(body) = 90", &[])
        .await?
        .get(0);
    assert_eq!(count, i64::from(rows));

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.copy+2/test]    COPY TO over a SELECT whose values are inlined
#[pgorm_macros::test]
async fn copy_out_of_a_select() -> Result<(), Error> {
    let ctx = TestContext::new("copy_out_of_a_select").await;
    let mut db = ctx.db.get().await?;
    db.execute(ITEMS, &[]).await?;
    db.execute(
        "INSERT INTO item VALUES (1, 'it''s', 3), (2, 'plain', 4), (3, 'it''s', 9)",
        &[],
    )
    .await?;

    let select = Query::select()
        .columns([n("id"), n("qty")])
        .from(n("item"))
        .and_where(Expr::col(n("name")).eq("it's"))
        .and_where(Expr::col(n("qty")).lt(5))
        .take();
    let out = copy_out_all(&mut db, &CopyToStatement::new(select, CopyCsv)).await?;
    assert_eq!(out, b"1,3\n");

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.copy+2/test]    each write is a COPY TO source through its RETURNING list
#[pgorm_macros::test]
async fn copy_out_of_writes_returning() -> Result<(), Error> {
    let ctx = TestContext::new("copy_out_of_writes_returning").await;
    let mut db = ctx.db.get().await?;
    db.execute(ITEMS, &[]).await?;
    db.execute("INSERT INTO item VALUES (1, 'one', 3), (2, 'two', 4)", &[])
        .await?;

    let insert = Query::insert()
        .into_table(n("item"))
        .columns([n("id"), n("name")])
        .values_panic([4.into(), "new".into()])
        .returning(Query::returning().columns([n("id"), n("qty")]))
        .to_owned();
    let copy = CopyToStatement::new(CopySource::try_from(insert)?, CopyText);
    assert_eq!(copy_out_all(&mut db, &copy).await?, b"4\t7\n");

    let update = Query::update()
        .table(n("item"))
        .value(n("qty"), Expr::col(n("qty")).add(10))
        .and_where(Expr::col(n("id")).eq(1))
        .returning(Query::returning().columns([n("id"), n("qty")]))
        .to_owned();
    let copy = CopyToStatement::new(CopySource::try_from(update)?, CopyCsv);
    assert_eq!(copy_out_all(&mut db, &copy).await?, b"1,13\n");

    let delete = Query::delete()
        .from_table(n("item"))
        .and_where(Expr::col(n("id")).eq(2))
        .returning(Query::returning().column(n("name")))
        .to_owned();
    let copy = CopyToStatement::new(CopySource::try_from(delete)?, CopyText);
    assert_eq!(copy_out_all(&mut db, &copy).await?, b"two\n");

    // `merge_action()` alone is the list a MERGE's rows are read through.
    db.execute("CREATE TABLE zeroed (id int)", &[]).await?;
    db.execute("INSERT INTO zeroed VALUES (1), (4)", &[])
        .await?;
    let merge = Query::merge(
        n("item"),
        n("zeroed"),
        Expr::col((n("item"), n("id"))).equals((n("zeroed"), n("id"))),
    )
    .when_matched(MergeUpdate::value(n("qty"), 0))
    .returning_action()
    .to_owned();
    let copy = CopyToStatement::new(CopySource::try_from(merge)?, CopyText);
    assert_eq!(copy_out_all(&mut db, &copy).await?, b"UPDATE\nUPDATE\n");

    assert_eq!(
        items(&db, "item").await?,
        vec![(1, Some("one".into()), 0), (4, Some("new".into()), 0)]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.copy+2/test]    a write without RETURNING is refused as a source before anything runs
#[pgorm_macros::test]
async fn copy_out_refuses_silent_write() -> Result<(), Error> {
    let ctx = TestContext::new("copy_out_refuses_silent_write").await;
    let db = ctx.db.get().await?;
    db.execute(ITEMS, &[]).await?;

    let insert = Query::insert()
        .into_table(n("item"))
        .columns([n("id")])
        .values_panic([1.into()])
        .to_owned();
    let refused = CopySource::try_from(insert).map_err(Error::from);
    assert!(
        matches!(
            refused,
            Err(Error::QueryBuilder(QueryError::CopyWithoutReturning {
                write: "INSERT"
            }))
        ),
        "{refused:?}"
    );
    assert_eq!(items(&db, "item").await?, vec![]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.copy+2/test]    a SELECT source's refused tie limit or window is refused before anything is sent
#[pgorm_macros::test]
async fn copy_out_checks_select_source() -> Result<(), Error> {
    let ctx = TestContext::new("copy_out_checks_select_source").await;
    let mut db = ctx.db.get().await?;
    db.execute(ITEMS, &[]).await?;
    db.execute(
        "INSERT INTO item VALUES (1, 'a', 3), (2, 'b', 3), (3, 'c', 4)",
        &[],
    )
    .await?;

    let tied = Query::select()
        .column(n("id"))
        .from(n("item"))
        .limit_with_ties(1)
        .to_owned();
    let refused = db
        .copy_out(&CopyToStatement::new(tied.clone(), CopyText))
        .await
        .map(drop);
    assert!(
        matches!(
            refused,
            Err(Error::QueryBuilder(QueryError::TiesWithoutOrder))
        ),
        "{refused:?}"
    );
    answers_in_step(&db).await?;

    let windowed = Query::select()
        .expr_window_name(Func::count(Expr::col(n("id"))), n("w"))
        .from(n("item"))
        .to_owned();
    db.transaction(async |tx| {
        let refused = tx
            .copy_out(&CopyToStatement::new(windowed, CopyCsv))
            .await
            .map(drop);
        assert!(
            matches!(
                refused,
                Err(Error::QueryBuilder(QueryError::WindowUndefined { ref name })) if name == "w"
            ),
            "{refused:?}"
        );
        // Nothing reached the server, so the transaction is not aborted.
        assert_eq!(tx.query_one("SELECT 1", &[]).await?.get::<_, i32>(0), 1);
        Ok::<_, Error>(())
    })
    .await
    .expect("the transaction commits");

    let ordered = tied.clone().order_by(n("qty"), Order::Asc).to_owned();
    let out = copy_out_all(&mut db, &CopyToStatement::new(ordered, CopyText)).await?;
    // Both rows tie on the one key, so their order is the server's.
    let mut ids: Vec<&[u8]> = out.split(|&byte| byte == b'\n').collect();
    ids.sort_unstable();
    assert_eq!(ids, [&b""[..], b"1", b"2"]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.copy+2/test]    a COPY TO refused at open leaves the connection as it was
#[pgorm_macros::test]
async fn copy_out_refused_at_open() -> Result<(), Error> {
    let ctx = TestContext::new("copy_out_refused_at_open").await;
    let mut db = ctx.db.get().await?;

    let error = db
        .copy_out(&CopyToStatement::new(n("missing"), CopyText))
        .await
        .expect_err("no such table");
    assert_eq!(
        sqlstate(&error),
        Some(&SqlState::UNDEFINED_TABLE),
        "{error}"
    );
    assert_eq!(db.query_one("SELECT 1", &[]).await?.get::<_, i32>(0), 1);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// Every answer a fresh connection gives is its own: a desynchronized one
/// would hand each statement the answer meant for the one before.
async fn answers_in_step(db: &DatabaseConnection) -> Result<(), Error> {
    for value in 1..=3_i32 {
        let row = db.query_one("SELECT $1::int4 + 0", &[&value]).await?;
        assert_eq!(row.get::<_, i32>(0), value);
    }
    Ok(())
}

// [spec:pgorm:req:conn.copy+2/test]    a COPY FROM refused at open closes its connection rather than leave it out of step
#[pgorm_macros::test]
async fn copy_in_refused_at_open_closes() -> Result<(), Error> {
    let ctx = TestContext::new("copy_in_refused_at_open").await;
    let mut db = ctx.db.get().await?;
    db.execute(ITEMS, &[]).await?;

    let error = db
        .copy_in(&CopyFromStatement::new(n("missing"), CopyText))
        .await
        .expect_err("no such table");
    assert_eq!(
        sqlstate(&error),
        Some(&SqlState::UNDEFINED_TABLE),
        "{error}"
    );
    let after = db
        .query_one("SELECT 1", &[])
        .await
        .expect_err("the connection is closed");
    assert!(
        matches!(&after, Error::Postgres(error) if error.is_closed()),
        "{after}"
    );
    drop(db);

    // Inside a transaction already aborted, the COPY is refused at Bind and
    // takes the transaction's connection with it.
    let mut db = ctx.db.get().await?;
    answers_in_step(&db).await?;
    let outcome = db
        .transaction(async |txn| {
            txn.execute("INSERT INTO item (id) VALUES (1)", &[]).await?;
            assert!(txn.execute("SELECT 1/0", &[]).await.is_err());
            let error = txn
                .copy_in(&CopyFromStatement::new(n("item"), CopyText))
                .await
                .expect_err("the transaction is aborted");
            assert_eq!(
                sqlstate(&error),
                Some(&SqlState::IN_FAILED_SQL_TRANSACTION),
                "{error}"
            );
            roll_back::<()>()
        })
        .await;
    assert_rolled_back(outcome);
    drop(db);

    // The pool lends a connection that answers in step, and nothing the
    // transaction wrote survived it.
    let db = ctx.db.get().await?;
    answers_in_step(&db).await?;
    assert_eq!(items(&db, "item").await?, []);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.copy+2/test]    a column list naming a generated column is refused at the start and closes the connection; with no list the generated columns are left out
#[pgorm_macros::test]
async fn copy_in_naming_a_generated_column_closes() -> Result<(), Error> {
    let ctx = TestContext::new("copy_in_generated_column").await;
    let mut db = ctx.db.get().await?;
    db.batch_execute(
        "CREATE TABLE priced (id int PRIMARY KEY, price int NOT NULL, \
         doubled int GENERATED ALWAYS AS (price * 2) STORED, \
         tripled int GENERATED ALWAYS AS (price * 3) VIRTUAL)",
    )
    .await?;

    for generated in ["doubled", "tripled"] {
        let copy = CopyFromStatement::new(n("priced"), CopyText)
            .columns([n("id"), n("price"), n(generated)])
            .to_owned();
        let error = db
            .copy_in(&copy)
            .await
            .expect_err("a generated column takes no data");
        assert_eq!(
            sqlstate(&error),
            Some(&SqlState::INVALID_COLUMN_REFERENCE),
            "{error}"
        );
        let after = db
            .query_one("SELECT 1", &[])
            .await
            .expect_err("the connection is closed");
        assert!(
            matches!(&after, Error::Postgres(error) if error.is_closed()),
            "{after}"
        );
        drop(db);
        db = ctx.db.get().await?;
        answers_in_step(&db).await?;
    }

    let whole = CopyFromStatement::new(n("priced"), CopyText);
    assert_eq!(copy_in_all(&mut db, &whole, &["1\t10\n"]).await?, 1);
    let row = db
        .query_one("SELECT doubled, tripled FROM priced WHERE id = 1", &[])
        .await?;
    assert_eq!((row.get::<_, i32>(0), row.get::<_, i32>(1)), (20, 30));
    let out = copy_out_all(&mut db, &CopyToStatement::new(n("priced"), CopyText)).await?;
    assert_eq!(String::from_utf8_lossy(&out), "1\t10\n");

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.copy+2/test]    an abandoned COPY TO stream leaves the connection usable
#[pgorm_macros::test]
async fn copy_out_dropped_early() -> Result<(), Error> {
    let ctx = TestContext::new("copy_out_dropped_early").await;
    let mut db = ctx.db.get().await?;

    let mut out = db
        .copy_out(&CopyToStatement::new(
            Query::select()
                .expr(Expr::raw("generate_series(1, 50000)"))
                .take(),
            CopyText,
        ))
        .await?;
    assert_eq!(out.next().await.transpose()?.as_deref(), Some(&b"1\n"[..]));
    drop(out);

    assert_eq!(db.query_one("SELECT 2", &[]).await?.get::<_, i32>(0), 2);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:def:sql.ast.copy+2/test]    every option each direction and format offers is one PostgreSQL 18 accepts
#[pgorm_macros::test]
async fn copy_every_offered_option_runs() -> Result<(), Error> {
    let ctx = TestContext::new("copy_every_offered_option").await;
    let mut db = ctx.db.get().await?;
    db.execute(ITEMS, &[]).await?;

    let ignore = CopyIgnore {
        reject_limit: NonZeroU32::new(5),
        log_verbosity: Some(CopyLogVerbosity::Silent),
    };

    db.transaction(async |txn| {
        // FREEZE needs the table truncated in the current subtransaction.
        txn.execute("TRUNCATE item", &[]).await?;

        let text = CopyFromStatement::new(n("item"), CopyText)
            .columns([n("id"), n("name"), n("qty")])
            .freeze()
            .delimiter('|')
            .null_string("~")
            .default_string("@")
            .header_match()
            .encoding("UTF8")
            .on_error_ignore(ignore)
            .and_where(Expr::col(n("id")).gt(0))
            .to_owned();
        assert_eq!(
            copy_in_all(txn, &text, &["id|name|qty\n1|~|@\n0|zero|1\n"]).await?,
            1
        );

        let csv = CopyFromStatement::new(n("item"), CopyCsv)
            .columns([n("id"), n("name"), n("qty")])
            .freeze()
            .delimiter(';')
            .null_string("N")
            .default_string("D")
            .header()
            .quote('\'')
            .escape('\\')
            .force_not_null([n("name")])
            .force_null([n("qty")])
            .encoding("UTF8")
            .on_error_ignore(CopyIgnore::default())
            .to_owned();
        assert_eq!(
            copy_in_all(txn, &csv, &["ignored header\n2;'N';D\n"]).await?,
            1
        );

        let all = CopyFromStatement::new(n("item"), CopyCsv)
            .force_not_null_all()
            .force_null_all()
            .to_owned();
        assert_eq!(copy_in_all(txn, &all, &["3,c,3\n"]).await?, 1);

        let binary_out = copy_out_all(txn, &CopyToStatement::new(n("item"), CopyBinary)).await?;
        txn.execute("TRUNCATE item", &[]).await?;
        let binary = CopyFromStatement::new(n("item"), CopyBinary)
            .columns([n("id"), n("name"), n("qty")])
            .freeze()
            .to_owned();
        let mut sink = txn.copy_in(&binary).await?;
        sink.send(Bytes::from(binary_out)).await?;
        assert_eq!(sink.finish().await?, 3);
        Ok::<_, Error>(())
    })
    .await
    .expect("every option's COPY commits");

    assert_eq!(
        items(&db, "item").await?,
        [
            (1, None, 7),
            (2, Some("N".to_owned()), 7),
            (3, Some("c".to_owned()), 3),
        ]
    );

    let text_out = CopyToStatement::new(n("item"), CopyText)
        .delimiter(',')
        .null_string("NULL")
        .header()
        .encoding("LATIN1")
        .to_owned();
    assert_eq!(
        copy_out_all(&mut db, &text_out).await?,
        b"id,name,qty\n1,NULL,7\n2,N,7\n3,c,3\n"
    );
    let csv_out = CopyToStatement::new(n("item"), CopyCsv)
        .delimiter('|')
        .null_string("")
        .header()
        .quote('\'')
        .escape('\\')
        .force_quote([n("name")])
        .encoding("UTF8")
        .to_owned();
    assert_eq!(
        copy_out_all(&mut db, &csv_out).await?,
        b"id|name|qty\n1||7\n2|'N'|7\n3|'c'|3\n"
    );
    let csv_all = CopyToStatement::new(n("item"), CopyCsv)
        .force_quote_all()
        .to_owned();
    assert_eq!(
        copy_out_all(&mut db, &csv_all).await?,
        b"\"1\",,\"7\"\n\"2\",\"N\",\"7\"\n\"3\",\"c\",\"3\"\n"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.copy+2/test]    ON_ERROR ignore skips a row that does not convert, and the count says so
#[pgorm_macros::test]
async fn copy_ignore_skips_bad_rows() -> Result<(), Error> {
    let ctx = TestContext::new("copy_ignore_skips_bad_rows").await;
    let mut db = ctx.db.get().await?;
    db.execute(ITEMS, &[]).await?;

    let copy = CopyFromStatement::new(n("item"), CopyText)
        .on_error_ignore(CopyIgnore::default())
        .to_owned();
    assert_eq!(
        copy_in_all(&mut db, &copy, &["1\ta\t1\nnot a number\tb\t2\n3\tc\t3\n"]).await?,
        2
    );
    assert_eq!(
        items(&db, "item").await?,
        [(1, Some("a".to_owned()), 1), (3, Some("c".to_owned()), 3)]
    );

    // Past the reject limit the COPY fails after all.
    let limited = CopyFromStatement::new(n("item"), CopyText)
        .on_error_ignore(CopyIgnore {
            reject_limit: NonZeroU32::new(1),
            log_verbosity: Some(CopyLogVerbosity::Verbose),
        })
        .to_owned();
    let error = copy_in_all(&mut db, &limited, &["x\ta\t1\ny\tb\t2\n"])
        .await
        .expect_err("two rejects exceed a limit of one");
    assert_eq!(
        sqlstate(&error),
        Some(&SqlState::INVALID_TEXT_REPRESENTATION),
        "{error}"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

#[derive(Clone, Debug, Default)]
struct Operations(Arc<Mutex<Vec<String>>>);

#[async_trait]
impl MetricsCollector for Operations {
    async fn record_query_success(
        &self,
        query: QueryContext<'_>,
        _duration: Duration,
        rows: Option<u64>,
    ) {
        self.0.lock().unwrap().push(format!(
            "ok:{}:{rows:?}:{}",
            query.operation(),
            query.sql().unwrap_or_default()
        ));
    }

    async fn record_query_error(
        &self,
        query: QueryContext<'_>,
        _duration: Duration,
        _error: &Error,
    ) {
        self.0
            .lock()
            .unwrap()
            .push(format!("error:{}", query.operation()));
    }

    async fn record_connection_acquired(&self, _duration: Duration) {}
    async fn record_connection_error(&self, _duration: Duration, _error: &Error) {}
    async fn record_transaction_begin(&self, _duration: Duration) {}
    async fn record_transaction_commit(&self, _duration: Duration) {}
    async fn record_transaction_rollback(&self, _duration: Duration) {}
}

// [spec:pgorm:req:metric.layer.copy/test]    the wrappers forward COPY and report it at open, with its text
// [spec:pgorm:req:metric.fingerprint+3/test]    a COPY is reported under copy_in and copy_out
#[pgorm_macros::test]
async fn instrumented_copy_reports_at_open() -> Result<(), Error> {
    let ctx = TestContext::new("instrumented_copy_reports").await;
    let metrics = Operations::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());
    let mut db = pool.get().await?;
    db.execute(ITEMS, &[]).await?;

    let copy_in = CopyFromStatement::new(n("item"), CopyText);
    assert_eq!(copy_in_all(&mut db, &copy_in, &["1\ta\t1\n"]).await?, 1);
    let copy_out = CopyToStatement::new(n("item"), CopyCsv);
    assert_eq!(copy_out_all(&mut db, &copy_out).await?, b"1,a,1\n");
    let mut doomed = pool.get().await?;
    assert!(
        doomed
            .copy_in(&CopyFromStatement::new(n("missing"), CopyText))
            .await
            .is_err()
    );
    drop(doomed);

    db.transaction(async |txn| {
        assert_eq!(copy_in_all(txn, &copy_in, &["2\tb\t2\n"]).await?, 1);
        assert_eq!(copy_out_all(txn, &copy_out).await?, b"1,a,1\n2,b,2\n");
        Ok::<_, Error>(())
    })
    .await
    .expect("the instrumented transaction commits");

    let recorded = metrics.0.lock().unwrap().clone();
    let copies: Vec<_> = recorded
        .iter()
        .filter(|event| event.contains("copy_"))
        .cloned()
        .collect();
    assert_eq!(
        copies,
        [
            format!("ok:copy_in:None:{copy_in}"),
            format!("ok:copy_out:None:{copy_out}"),
            "error:copy_in".to_owned(),
            format!("ok:copy_in:None:{copy_in}"),
            format!("ok:copy_out:None:{copy_out}"),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
