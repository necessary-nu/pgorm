#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, bakery_chain::*, setup::*};
use futures::TryStreamExt;
use pgorm::{ItemsAndPagesNumber, entity::prelude::*};
use pgorm_query::{Value, Values};
use pretty_assertions::assert_eq;
use std::num::NonZeroU64;

const BAKERIES: [(&str, f64); 7] = [
    ("Alpha Bakery", 1.0),
    ("Bravo Bakery", 2.0),
    ("Charlie Bakery", 3.0),
    ("Delta Bakery", 4.0),
    ("Echo Bakery", 5.0),
    ("Foxtrot Bakery", 6.0),
    ("Golf Bakery", 7.0),
];

async fn seed(db: &DatabaseConnection) -> Result<(), Error> {
    for (name, margin) in BAKERIES {
        bakery::ActiveModel {
            name: set(name),
            profit_margin: set(margin),
            ..Default::default()
        }
        .insert(db)
        .await?;
    }

    Ok(())
}

fn names(models: &[bakery::Model]) -> Vec<&str> {
    models.iter().map(|m| m.name.as_str()).collect()
}

/// A page size written as a literal. Library code needs no such helper — the
/// `NonZeroU64` is the check — so the one `expect` is confined to here.
fn page_size(size: u64) -> NonZeroU64 {
    NonZeroU64::new(size).expect("page size is non-zero")
}

const RAW_ALL: &str = r#"SELECT "id", "name", "profit_margin" FROM "bakery" ORDER BY "id" ASC"#;

// [spec:pgorm:def:exec.paginator+3/test]    `PaginatorTrait::paginate` is
// reachable from every builder-backed selector shape and returns no `Result`
// on any of them, and `count` pages at `NonZeroU64::MIN`
// [spec:pgorm:sem:exec.paginator.fetch+3/test]    zero-indexed pages, an
// independent page cursor, and `advance` moving it without fetching
#[pgorm_macros::test]
async fn paginator_fetch_page() -> Result<(), Error> {
    let ctx = TestContext::new("paginator_tests_fetch_page").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    let mut paginator = Bakery::find()
        .order_by_asc(bakery::Column::Id)
        .paginate(&db, page_size(3));

    // Pages are zero-indexed and the trailing page is partial.
    assert_eq!(
        names(&paginator.fetch_page(0).await?),
        ["Alpha Bakery", "Bravo Bakery", "Charlie Bakery"]
    );
    assert_eq!(
        names(&paginator.fetch_page(1).await?),
        ["Delta Bakery", "Echo Bakery", "Foxtrot Bakery"]
    );
    assert_eq!(names(&paginator.fetch_page(2).await?), ["Golf Bakery"]);
    assert!(paginator.fetch_page(3).await?.is_empty());

    // `fetch_page` neither consults nor advances the paginator's own cursor.
    assert_eq!(paginator.cur_page(), 0);
    assert_eq!(
        names(&paginator.fetch().await?),
        names(&paginator.fetch_page(0).await?)
    );

    // `advance` moves the counter without fetching; `fetch` then reads the new
    // page. The name is the point: nothing here reads the database.
    paginator.advance();
    assert_eq!(paginator.cur_page(), 1);
    assert_eq!(
        names(&paginator.fetch().await?),
        ["Delta Bakery", "Echo Bakery", "Foxtrot Bakery"]
    );
    paginator.advance();
    paginator.advance();
    assert_eq!(paginator.cur_page(), 3);
    assert!(paginator.fetch().await?.is_empty());

    // `PaginatorTrait` is implemented for `Selector<S>` too.
    let names_only: Vec<String> = Bakery::find()
        .select(bakery::Column::Name)
        .order_by_asc(bakery::Column::Id)
        .into_tuple::<String>()
        .paginate(&db, page_size(2))
        .fetch_page(1)
        .await?;
    assert_eq!(names_only, ["Charlie Bakery", "Delta Bakery"]);

    // ... and for `SelectGraph<E, S>`, via its own selector.
    let joined: Vec<(bakery::Model, Option<baker::Model>)> = Bakery::graph()
        .related_maybe::<Baker>()
        .order_by_asc(bakery::Column::Id)
        .paginate(&db, page_size(2))
        .fetch_page(0)
        .await?;
    assert_eq!(joined.len(), 2);
    assert_eq!(joined[0].1, None);

    // `count` is `num_items` over a paginator of page size one.
    assert_eq!(Bakery::find().count(&db).await?, 7);

    // A page whose offset would not fit a `u64` is refused before any SQL is
    // built, rather than wrapping to a small offset or panicking on overflow.
    let overflowing = Bakery::find()
        .paginate(&db, NonZeroU64::MAX)
        .fetch_page(2)
        .await;
    assert!(
        matches!(&overflowing, Err(Error::Query(_))),
        "unexpected result: {overflowing:?}"
    );
    assert!(
        overflowing
            .unwrap_err()
            .to_string()
            .contains("largest representable offset")
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:exec.paginator.count/test]    counting strips limit, offset
// and ORDER BY; page count is ceiling division and is never cached
#[pgorm_macros::test]
async fn paginator_count() -> Result<(), Error> {
    let ctx = TestContext::new("paginator_tests_count").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    // Zero items yield zero pages.
    let empty = Bakery::find().paginate(&db, page_size(3));
    assert_eq!(empty.num_items().await?, 0);
    assert_eq!(empty.num_pages().await?, 0);

    seed(&db).await?;

    // A partial trailing page still counts as a page: 7 items / 3 == 3 pages.
    let paginator = Bakery::find()
        .order_by_asc(bakery::Column::Id)
        .paginate(&db, page_size(3));
    assert_eq!(paginator.num_items().await?, 7);
    assert_eq!(paginator.num_pages().await?, 3);

    let ItemsAndPagesNumber {
        number_of_items,
        number_of_pages,
    } = paginator.num_items_and_pages().await?;
    assert_eq!(number_of_items, 7);
    assert_eq!(number_of_pages, 3);

    // An exact multiple does not gain a trailing page.
    assert_eq!(
        Bakery::find()
            .paginate(&db, page_size(7))
            .num_pages()
            .await?,
        1
    );
    assert_eq!(
        Bakery::find()
            .paginate(&db, page_size(1))
            .num_pages()
            .await?,
        7
    );

    // The count subquery drops limit, offset and ORDER BY, so a query that
    // would return two rows still counts every matching row.
    let windowed = Bakery::find()
        .order_by_desc(bakery::Column::Id)
        .limit(2)
        .offset(1)
        .paginate(&db, page_size(3));
    assert_eq!(windowed.fetch_page(0).await?.len(), 3);
    assert_eq!(windowed.num_items().await?, 7);

    // Counts are re-run, not cached: a row inserted after the first count is
    // visible to the second.
    bakery::ActiveModel {
        name: set("Hotel Bakery"),
        profit_margin: set(8.0),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    assert_eq!(paginator.num_items().await?, 8);
    assert_eq!(paginator.num_pages().await?, 3);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:exec.paginator.iterate/test]    `fetch_and_next` terminates
// only on an empty page, and `into_stream` wraps that same loop
#[pgorm_macros::test]
async fn paginator_iterate() -> Result<(), Error> {
    let ctx = TestContext::new("paginator_tests_iterate").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    let mut paginator = Bakery::find()
        .order_by_asc(bakery::Column::Id)
        .paginate(&db, page_size(3));

    let mut pages = Vec::new();
    while let Some(page) = paginator.fetch_and_next().await? {
        pages.push(page.len());
    }
    assert_eq!(pages, [3, 3, 1]);
    // The counter is advanced by the empty fetch that ended the loop too.
    assert_eq!(paginator.cur_page(), 4);

    // A result set that is an exact multiple of `page_size` costs one extra
    // query returning zero rows before termination is detected.
    let mut exact = Bakery::find()
        .order_by_asc(bakery::Column::Id)
        .paginate(&db, page_size(7));
    assert_eq!(exact.fetch_and_next().await?.map(|p| p.len()), Some(7));
    assert_eq!(exact.fetch_and_next().await?.map(|p| p.len()), None);
    assert_eq!(exact.cur_page(), 2);

    // `into_stream` yields one item per non-empty page and then ends.
    let streamed: Vec<Vec<bakery::Model>> = Bakery::find()
        .order_by_asc(bakery::Column::Id)
        .paginate(&db, page_size(3))
        .into_stream()
        .try_collect()
        .await?;
    assert_eq!(
        streamed.iter().map(|p| p.len()).collect::<Vec<_>>(),
        [3, 3, 1]
    );
    assert_eq!(
        names(&streamed.concat()),
        BAKERIES.map(|(name, _)| name).to_vec()
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:exec.crud+1/test]    `Select::from_raw_sql` builds a
// `SelectorRaw` from a raw statement plus `Values`
// [spec:pgorm:sem:exec.paginator.raw+7/test]    a parsed single `SELECT` is
// wrapped whole as a subquery, so its own clauses survive paging
#[pgorm_macros::test]
async fn paginator_raw() -> Result<(), Error> {
    let ctx = TestContext::new("paginator_tests_raw").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    // No bind values.
    let mut paginator = Bakery::find()
        .from_raw_sql(RAW_ALL.to_owned(), Values(Vec::new()))
        .paginate(&db, page_size(3))?;

    assert_eq!(
        names(&paginator.fetch_page(0).await?),
        ["Alpha Bakery", "Bravo Bakery", "Charlie Bakery"]
    );
    assert_eq!(names(&paginator.fetch_page(2).await?), ["Golf Bakery"]);
    assert_eq!(paginator.num_items().await?, 7);
    assert_eq!(paginator.num_pages().await?, 3);
    assert_eq!(paginator.fetch_and_next().await?.map(|p| p.len()), Some(3));

    // With bind values: the `$1` marker keeps both its number and its value.
    let filtered = Bakery::find()
        .from_raw_sql(
            r#"SELECT "id", "name", "profit_margin" FROM "bakery" WHERE "profit_margin" > $1 ORDER BY "id" ASC"#
                .to_owned(),
            Values(vec![Value::Double(Some(4.0))]),
        )
        .paginate(&db, page_size(2))?;

    assert_eq!(filtered.num_items().await?, 3);
    assert_eq!(
        names(&filtered.fetch_page(0).await?),
        ["Echo Bakery", "Foxtrot Bakery"]
    );
    assert_eq!(names(&filtered.fetch_page(1).await?), ["Golf Bakery"]);

    // A `WITH ... SELECT` is one `SelectStmt` to the parser, so a CTE pages
    // like any other statement — the case byte-slicing off a `SELECT` keyword
    // could never handle.
    let cte = Bakery::find()
        .from_raw_sql(
            format!(r#"WITH t AS ({RAW_ALL}) SELECT * FROM t ORDER BY "id" ASC"#),
            Values(Vec::new()),
        )
        .paginate(&db, page_size(3))?;
    assert_eq!(cte.num_items().await?, 7);
    assert_eq!(cte.num_pages().await?, 3);
    assert_eq!(
        names(&cte.fetch_page(1).await?),
        ["Delta Bakery", "Echo Bakery", "Foxtrot Bakery"]
    );
    assert_eq!(names(&cte.fetch_page(2).await?), ["Golf Bakery"]);

    // The statement is taken at the extent the parser reports, so leading
    // whitespace, a leading comment and a terminating `;` all page fine.
    let decorated = Bakery::find()
        .from_raw_sql(format!("  -- the lot\n  {RAW_ALL} ; "), Values(Vec::new()))
        .paginate(&db, page_size(3))?;
    assert_eq!(decorated.num_items().await?, 7);
    assert_eq!(names(&decorated.fetch_page(2).await?), ["Golf Bakery"]);

    // Wrapping puts `LIMIT`/`OFFSET` outside the caller's own clauses instead
    // of colliding with them, so a statement that already limits still pages.
    let capped = Bakery::find()
        .from_raw_sql(format!(r#"{RAW_ALL} LIMIT 4"#), Values(Vec::new()))
        .paginate(&db, page_size(3))?;
    assert_eq!(capped.num_items().await?, 4);
    assert_eq!(names(&capped.fetch_page(1).await?), ["Delta Bakery"]);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:exec.paginator.raw+7/test]    markers the caller did not
// number in first-reference order still read the values the caller meant,
// because the wrapper renumbers them into its own parameter space instead of
// requiring the caller's numbering to be usable where the fragment lands
#[pgorm_macros::test]
async fn paginator_raw_renumbers_markers() -> Result<(), Error> {
    let ctx = TestContext::new("paginator_tests_raw_renumbers").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    // Read out of order: `$2` is still the second value supplied, and `$1` the
    // first, after the wrapper has renumbered both.
    let swapped = r#"SELECT "id", "name", "profit_margin" FROM "bakery" WHERE "profit_margin" <= $1 AND "profit_margin" >= $2 ORDER BY "id" ASC"#;
    let bounds = Values(vec![Value::Double(Some(5.0)), Value::Double(Some(3.0))]);

    let paginator = Bakery::find()
        .from_raw_sql(swapped.to_owned(), bounds.clone())
        .paginate(&db, page_size(2))?;

    assert_eq!(paginator.num_items().await?, 3);
    assert_eq!(paginator.num_pages().await?, 2);
    assert_eq!(
        names(&paginator.fetch_page(0).await?),
        ["Charlie Bakery", "Delta Bakery"]
    );
    assert_eq!(names(&paginator.fetch_page(1).await?), ["Echo Bakery"]);

    // The same statement run unpaginated is the oracle: paging changed which
    // rows came back in no way at all.
    assert_eq!(
        names(
            &Bakery::find()
                .from_raw_sql(swapped.to_owned(), bounds)
                .all(&db)
                .await?
        ),
        ["Charlie Bakery", "Delta Bakery", "Echo Bakery"]
    );

    // One value read twice becomes two parameters, both holding it.
    let twice = r#"SELECT "id", "name", "profit_margin" FROM "bakery" WHERE "profit_margin" >= $1 AND "profit_margin" < $1::float8 + 3 ORDER BY "id" ASC"#;
    let floor = Values(vec![Value::Double(Some(2.0))]);

    let repeated = Bakery::find()
        .from_raw_sql(twice.to_owned(), floor.clone())
        .paginate(&db, page_size(2))?;

    assert_eq!(repeated.num_items().await?, 3);
    assert_eq!(
        names(&repeated.fetch_page(0).await?),
        ["Bravo Bakery", "Charlie Bakery"]
    );
    assert_eq!(names(&repeated.fetch_page(1).await?), ["Delta Bakery"]);
    assert_eq!(
        names(
            &Bakery::find()
                .from_raw_sql(twice.to_owned(), floor)
                .all(&db)
                .await?
        ),
        ["Bravo Bakery", "Charlie Bakery", "Delta Bakery"]
    );

    // A value the statement never reads is refused by `paginate` itself,
    // naming it: there is no paginator to ask, rather than a paginator that
    // would have reported it once per reader.
    let spare = Bakery::find()
        .from_raw_sql(RAW_ALL.to_owned(), Values(vec![Value::Double(Some(1.0))]))
        .paginate(&db, page_size(3))
        .expect_err("an unread bind value was not refused");

    assert!(
        matches!(spare, Error::Query(_)),
        "unexpected error: {spare:?}"
    );
    assert!(
        spare
            .to_string()
            .contains("given 1 bind values when nothing in it reads $1"),
        "{spare} does not name the unread value"
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:exec.paginator.raw+7/test]    anything that is not one
// row-returning `SELECT` is an `Error::Query` from `paginate` itself, naming
// what it parsed as — so no paginator over it is ever handed back
// [spec:pgorm:def:exec.paginator+3/test]    `SelectorRaw::paginate` is the
// fallible one; `count` on the same source reports the same refusal
#[pgorm_macros::test]
async fn paginator_raw_rejects_non_select() -> Result<(), Error> {
    let ctx = TestContext::new("paginator_tests_raw_rejects").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    // Statements too short to hold a keyword, scripts, DML, DDL and a
    // row-returning-looking `SELECT ... INTO`, each named for what it is.
    let cases = [
        ("", "holding 0 statements"),
        ("   ", "holding 0 statements"),
        ("SEL", "PostgreSQL rejects"),
        ("SELECT 1; SELECT 2", "holding 2 statements"),
        (
            r#"INSERT INTO "bakery" ("name") VALUES ('x')"#,
            "InsertStmt",
        ),
        (r#"UPDATE "bakery" SET "name" = 'x'"#, "UpdateStmt"),
        (r#"DELETE FROM "bakery""#, "DeleteStmt"),
        (r#"CREATE TABLE "t" ("a" int)"#, "CreateStmt"),
        (r#"SELECT * INTO "t" FROM "bakery""#, "SELECT ... INTO"),
    ];

    for (stmt, expected) in cases {
        let raw = || Bakery::find().from_raw_sql(stmt.to_owned(), Values(Vec::new()));

        // Both entry points on the raw selector refuse it, and both refuse it
        // at the call the caller made rather than at some later read.
        for reported in [
            raw().paginate(&db, page_size(3)).err(),
            raw().count(&db).await.err(),
        ] {
            let reported = reported.unwrap_or_else(|| panic!("{stmt:?} was not refused"));
            assert!(
                matches!(reported, Error::Query(_)),
                "{stmt:?}: unexpected error: {reported:?}"
            );
            assert!(
                reported.to_string().contains(expected),
                "{stmt:?}: {reported} does not name {expected:?}"
            );
        }
    }

    // Refusal is the paginator's, not the server's: the table is untouched.
    assert_eq!(Bakery::find().count(&db).await?, 7);

    drop(db);
    ctx.delete().await;

    Ok(())
}

/// Statements a `$N` walk over the text would corrupt, each paired with the
/// values it binds. Every one carries a `$99` that is not a marker at all —
/// comment or string text — or brackets a lexer that reads `[` as a quote
/// delimiter would swallow. Each selects a single text column so the direct and
/// paginated answers compare directly.
fn token_forms() -> Vec<(&'static str, Values)> {
    let one = || Values(vec![Value::Int(Some(7))]);
    vec![
        (
            "SELECT $1::int4::text AS v /* $99 is only a comment */",
            one(),
        ),
        // PostgreSQL nests block comments, so the inner close does not end it.
        (
            "SELECT $1::int4::text AS v /* outer /* $99 */ still outer */",
            one(),
        ),
        ("SELECT $1::int4::text AS v -- $99 to end of line", one()),
        (r#"SELECT ($1::int4::text || $$ $99 '"` $$) AS v"#, one()),
        ("SELECT ($1::int4::text || $tag$ $99 $$ x$tag$) AS v", one()),
        ("SELECT ($1::int4::text || ' it''s $99') AS v", one()),
        (r"SELECT ($1::int4::text || E' a\'b $99') AS v", one()),
        // The same value read twice, and two values read out of order.
        ("SELECT ($1::int4::text || $1::int4::text) AS v", one()),
        (
            "SELECT ($2::int4::text || $1::int4::text) AS v",
            Values(vec![Value::Int(Some(7)), Value::Int(Some(3))]),
        ),
        // Subscripts around and after markers.
        (
            "SELECT (ARRAY[$1::int4, 8])[$2::int4]::text AS v",
            Values(vec![Value::Int(Some(7)), Value::Int(Some(2))]),
        ),
    ]
}

// [spec:pgorm:sem:exec.paginator.raw+7/test]    the caller's statement is sent
// whole, so comments, dollar quotes, string literals and subscripts read the
// same paginated as they do direct, and the markers keep their values
#[pgorm_macros::test]
async fn paginator_raw_token_forms() -> Result<(), Error> {
    let ctx = TestContext::new("paginator_tests_raw_token_forms").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    for (sql, values) in token_forms() {
        let direct = (sql, values.clone())
            .into_tuple::<String>()
            .one(&db)
            .await?;

        let paginator = (sql, values.clone())
            .into_tuple::<String>()
            .paginate(&db, page_size(10))?;

        assert_eq!(
            paginator.fetch_page(0).await?,
            vec![direct.clone()],
            "{sql:?} paged differently from the direct answer {direct:?}"
        );
        assert_eq!(paginator.num_items().await?, 1, "{sql:?} counted wrong");
        assert_eq!(paginator.num_pages().await?, 1, "{sql:?} paged wrong");
    }

    // The two shapes as first reported: a comment holding a marker number
    // beyond the values, which a text walk read as a marker and indexed past
    // the end of; and a dollar-quoted string a text walk rewrote into
    // syntactically invalid SQL.
    let commented = (
        "SELECT $1::int4 AS n /* $99 is only a comment */",
        Values(vec![Value::Int(Some(1))]),
    );
    assert_eq!(
        commented
            .clone()
            .into_tuple::<i32>()
            .paginate(&db, page_size(10))?
            .fetch_page(0)
            .await?,
        vec![commented.into_tuple::<i32>().one(&db).await?]
    );

    let quoted = (
        "SELECT $1::int4 AS n, $$hello$$ AS msg",
        Values(vec![Value::Int(Some(1))]),
    );
    assert_eq!(
        quoted
            .clone()
            .into_tuple::<(i32, String)>()
            .paginate(&db, page_size(10))?
            .fetch_page(0)
            .await?,
        vec![quoted.into_tuple::<(i32, String)>().one(&db).await?]
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:exec.paginator.raw+7/test]    a marker with no value behind
// it is an `Error::Query` naming it, returned by `paginate` rather than an
// index past the end of the values — and rather than a paginator that exists
// but cannot page
#[pgorm_macros::test]
async fn paginator_raw_rejects_unbound_marker() -> Result<(), Error> {
    let ctx = TestContext::new("paginator_tests_raw_unbound").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    // Here the `$99` really is a marker: it is not inside a comment.
    let unbound = || {
        (
            "SELECT $1::int4 AS n, $99::int4 AS m",
            Values(vec![Value::Int(Some(7))]),
        )
            .into_tuple::<(i32, i32)>()
    };

    for reported in [
        unbound().paginate(&db, page_size(10)).err(),
        unbound().count(&db).await.err(),
    ] {
        let reported = reported.expect("an unbound marker was not refused");
        assert!(
            matches!(reported, Error::Query(_)),
            "unexpected error: {reported:?}"
        );
        assert!(
            reported
                .to_string()
                .contains("reading $99 when 1 bind value was supplied"),
            "{reported} does not name the marker"
        );
    }

    // Refusal is the paginator's, not the server's: the table is untouched.
    assert_eq!(Bakery::find().count(&db).await?, 7);

    drop(db);
    ctx.delete().await;

    Ok(())
}
