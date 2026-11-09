//! `prql!`-produced statements against a live server, with bound parameters.
//!
//! The macro's own suite proves the SQL text and the Values shape; what it
//! cannot prove is that the placeholders and their values meet again at the
//! server — that `$1` reused in two clauses binds one value in both, and
//! that the rows coming back decode. Those are semantics, so they are
//! asserted against PostgreSQL with decoded rows.
//!
//! The same goes for the SQL prqlc writes for a grouping or a deduplication.
//! A whole-row key, a literal key and a dropped DISTINCT ON key all pass the
//! macro's grammar check whether or not the server will run them, so the
//! pinned compiler's handling of each is proved here by the rows that come
//! back.
//!
//! Run the test locally:
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:54329 cargo test --test prql_macro_live_tests
#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::TestContext;
use pgorm::{
    ConnectionTrait, DecodeRaw, Error, FromQueryResult, SelectModel, SelectorRaw, Values, prql, sql,
};
use pretty_assertions::assert_eq;

#[derive(FromQueryResult, Debug, PartialEq)]
struct Invoice {
    id: i32,
    billing_city: String,
    total: i64,
}

const SCHEMA: &str = sql!(
    "CREATE TABLE invoice (id int primary key, billing_city text not null, total bigint not null);
     INSERT INTO invoice (id, billing_city, total) VALUES
         (1, 'Berlin', 50), (2, 'Berlin', 120), (3, 'Oslo', 200), (4, 'Oslo', 80);"
);

fn selector(sql: &str, values: Values) -> SelectorRaw<SelectModel<Invoice>> {
    (sql, values).into_model::<Invoice>()
}

// [spec:pgorm:def:macros.prql+2/test]    the expansion binds its argument end to end
#[pgorm_macros::test]
async fn bound_param_reaches_the_server() -> Result<(), Error> {
    let ctx = TestContext::new("prql_macro_bound_param").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SCHEMA).await?;

    let (query, values) = prql!("from invoice | filter total > $1 | sort id", 100_i64);
    let rows = selector(query, values).all(&db).await?;

    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![2, 3]
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:macros.prql.census/test]    `$1` twice, one value, both clauses
#[pgorm_macros::test]
async fn reused_placeholder_binds_once_live() -> Result<(), Error> {
    let ctx = TestContext::new("prql_macro_reused_placeholder").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SCHEMA).await?;

    let (query, values) = prql!(
        "from invoice | filter total > $1 | filter id != $1 | sort id",
        100_i64,
    );
    assert_eq!(query.matches("$1").count(), 2);
    assert_eq!(values.0.len(), 1);

    // total > 100 keeps ids 2 and 3; id != 100 excludes nothing.
    let rows = selector(query, values).all(&db).await?;
    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![2, 3]
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:macros.prql+2/test]    two placeholders, two values, right slots
#[pgorm_macros::test]
async fn arguments_land_in_placeholder_order() -> Result<(), Error> {
    let ctx = TestContext::new("prql_macro_placeholder_order").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SCHEMA).await?;

    let city = "Oslo".to_owned();
    let (query, values) = prql!(
        "from invoice | filter billing_city == $1 | filter total < $2 | sort id",
        city,
        150_i64,
    );
    let rows = selector(query, values).all(&db).await?;

    assert_eq!(
        rows,
        vec![Invoice {
            id: 4,
            billing_city: "Oslo".to_owned(),
            total: 80,
        }]
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:macros.prql.sstring/test]    an s-string's SQL runs as written
#[pgorm_macros::test]
async fn sstring_survives_to_execution() -> Result<(), Error> {
    let ctx = TestContext::new("prql_macro_sstring_live").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SCHEMA).await?;

    #[derive(FromQueryResult)]
    struct City {
        lowered: String,
    }

    let (query, values) = prql!(
        r#"from invoice | filter total > $1 | derive lowered = s"lower(billing_city)" | select {lowered}"#,
        150_i64,
    );
    let rows = (query, values).into_model::<City>().all(&db).await?;

    assert_eq!(
        rows.iter()
            .map(|row| row.lowered.as_str())
            .collect::<Vec<_>>(),
        vec!["oslo"]
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

/// Six rows, three of them distinct, repeated one, two and three times.
///
/// No key: grouping a relation by `this` groups by its whole row, which
/// counts duplicates, and `prql!` declares no columns, so prqlc knows none of
/// these.
const TALLY: &str = sql!(
    "CREATE TABLE tally (fruit text not null, colour text not null, weight int not null);
     INSERT INTO tally (fruit, colour, weight) VALUES
         ('apple', 'red', 1), ('apple', 'red', 1), ('apple', 'green', 2),
         ('pear', 'green', 3), ('pear', 'green', 3), ('pear', 'green', 3);"
);

// [spec:pgorm:def:macros.prql+2/test]    grouping an undeclared relation by
// `this` writes a qualified whole-row key PostgreSQL can read
#[pgorm_macros::test]
async fn grouping_undeclared_rows_counts_duplicates() -> Result<(), Error> {
    let ctx = TestContext::new("prql_macro_group_whole_row").await;
    let db = ctx.db.get().await?;
    db.batch_execute(TALLY).await?;

    #[derive(FromQueryResult, Debug, PartialEq)]
    struct Count {
        n: i64,
    }

    let (query, values) =
        prql!("from tally | group this (aggregate {n = count this}) | select {n} | sort n");
    assert_eq!(
        query,
        "SELECT COUNT(*) AS n FROM tally GROUP BY tally.* ORDER BY n"
    );
    let rows = (query, values).into_model::<Count>().all(&db).await?;

    assert_eq!(rows, vec![Count { n: 1 }, Count { n: 2 }, Count { n: 3 }]);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:macros.prql+2/test]    a literal grouping key is projected but
// left out of GROUP BY, where PostgreSQL would read `5` as a column position;
// a grouping made only of literals keeps its empty-input answer
#[pgorm_macros::test]
async fn literal_group_key_is_not_a_position() -> Result<(), Error> {
    let ctx = TestContext::new("prql_macro_literal_group_key").await;
    let db = ctx.db.get().await?;
    db.batch_execute(TALLY).await?;

    #[derive(FromQueryResult, Debug, PartialEq)]
    struct Group {
        fruit: String,
        colour: String,
        x: i32,
        n: i64,
    }

    let (query, values) = prql!(
        "from tally | select {fruit, colour} | derive {x = 5} \
         | group this (aggregate {n = count this}) | sort {fruit, colour}"
    );
    assert_eq!(
        query,
        "SELECT fruit, colour, 5 AS x, COUNT(*) AS n FROM tally GROUP BY fruit, colour ORDER BY fruit, colour"
    );
    let rows = (query, values).into_model::<Group>().all(&db).await?;
    let group = |fruit: &str, colour: &str, n| Group {
        fruit: fruit.to_owned(),
        colour: colour.to_owned(),
        x: 5,
        n,
    };
    assert_eq!(
        rows,
        vec![
            group("apple", "green", 1),
            group("apple", "red", 2),
            group("pear", "green", 3),
        ]
    );

    #[derive(FromQueryResult, Debug, PartialEq)]
    struct Whole {
        x: i32,
        n: i64,
    }

    // Grouped by a constant alone, a non-empty input is one group and an
    // empty one is none, where an ungrouped aggregate would answer one row.
    let only_literals = |floor: i32| {
        prql!(
            "from tally | filter weight > $1 | derive {x = 5} | group {x} (aggregate {n = count this})",
            floor,
        )
    };
    let (query, values) = only_literals(0);
    assert_eq!(
        query,
        "SELECT 5 AS x, COUNT(*) AS n FROM tally WHERE weight > $1 HAVING COUNT(*) > 0"
    );
    let rows = (query, values).into_model::<Whole>().all(&db).await?;
    assert_eq!(rows, vec![Whole { x: 5, n: 6 }]);

    let (query, values) = only_literals(100);
    let rows = (query, values).into_model::<Whole>().all(&db).await?;
    assert_eq!(rows, vec![]);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:macros.prql+2/test]    a computed DISTINCT ON key, or a computed
// sort inside the group, that the projection drops is ordered by its
// expression rather than by an alias the SELECT no longer carries
#[pgorm_macros::test]
async fn dropped_distinct_key_still_orders_rows() -> Result<(), Error> {
    let ctx = TestContext::new("prql_macro_dropped_distinct_key").await;
    let db = ctx.db.get().await?;
    db.batch_execute(sql!(
        "CREATE TABLE cake (id int primary key, name text not null);
         INSERT INTO cake (id, name) VALUES
             (1, 'sponge'), (2, 'sponge'), (3, 'torte'), (4, 'sponge'),
             (5, 'torte'), (6, 'tart'), (7, 'tart');"
    ))
    .await?;

    #[derive(FromQueryResult, Debug, PartialEq)]
    struct Cake {
        name: String,
        id: i32,
    }

    // The newest cake for each residue of its id modulo 3: 6 of {3, 6}, 7 of
    // {1, 4, 7} and 5 of {2, 5}. The key `x` is not in the output.
    let (query, values) = prql!(
        "from cake | derive {x = id % 3} | group {x} (sort {-id} | take 1) | select {name, id}"
    );
    assert_eq!(
        query,
        "SELECT DISTINCT ON (id % 3) name, id FROM cake ORDER BY id % 3, id DESC"
    );
    let mut rows = (query, values).into_model::<Cake>().all(&db).await?;
    rows.sort_by_key(|cake| cake.id);
    let cake = |name: &str, id| Cake {
        name: name.to_owned(),
        id,
    };
    assert_eq!(
        rows,
        vec![cake("torte", 5), cake("tart", 6), cake("tart", 7)]
    );

    #[derive(FromQueryResult, Debug, PartialEq)]
    struct Id {
        id: i32,
    }

    // A computed sort inside the group reaches the same ORDER BY: the newest
    // cake of each name, ordered by an expression nothing projects.
    let (query, values) = prql!("from cake | group {name} (sort {id * -1} | take 1) | select {id}");
    assert_eq!(
        query,
        "SELECT DISTINCT ON (name) id FROM cake ORDER BY name, id * -1"
    );
    let mut rows = (query, values).into_model::<Id>().all(&db).await?;
    rows.sort_by_key(|row| row.id);
    assert_eq!(rows, vec![Id { id: 4 }, Id { id: 5 }, Id { id: 7 }]);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:macros.prql+2/test]    a named column read beside a whole-row
// key is written as a key too, since PostgreSQL reads a column outside an
// aggregate only when it is grouped by name
#[pgorm_macros::test]
async fn named_column_beside_whole_row_key() -> Result<(), Error> {
    let ctx = TestContext::new("prql_macro_whole_row_named_column").await;
    let db = ctx.db.get().await?;
    db.batch_execute(TALLY).await?;

    #[derive(FromQueryResult, Debug, PartialEq)]
    struct FruitCount {
        fruit: String,
        n: i64,
    }

    // Each distinct row is its own group, so apple appears once per colour.
    let (query, values) = prql!(
        "from tally | group this (aggregate {n = count this}) | select {fruit, n} | sort {fruit, n}"
    );
    assert_eq!(
        query,
        "SELECT fruit, COUNT(*) AS n FROM tally GROUP BY fruit, tally.* ORDER BY fruit, n"
    );
    let rows = (query, values).into_model::<FruitCount>().all(&db).await?;
    let count = |fruit: &str, n| FruitCount {
        fruit: fruit.to_owned(),
        n,
    };
    assert_eq!(
        rows,
        vec![count("apple", 1), count("apple", 2), count("pear", 3)]
    );

    #[derive(FromQueryResult, Debug, PartialEq)]
    struct Count {
        n: i64,
    }

    // A column read only by HAVING is written as a key all the same.
    let (query, values) = prql!(
        r#"from tally | group this (aggregate {n = count this}) | filter fruit == "apple" | select {n} | sort n"#
    );
    assert_eq!(
        query,
        "SELECT COUNT(*) AS n FROM tally GROUP BY fruit, tally.* HAVING fruit = 'apple' ORDER BY n"
    );
    let rows = (query, values).into_model::<Count>().all(&db).await?;
    assert_eq!(rows, vec![Count { n: 1 }, Count { n: 2 }]);

    drop(db);
    ctx.delete().await;

    Ok(())
}

/// Rows of `sql` as `(i32, i64)` pairs, sorted: the server's answer to SQL
/// this suite wrote, or to SQL `prql!` emitted.
async fn pairs(db: &impl ConnectionTrait, sql: &str) -> Result<Vec<(i32, i64)>, Error> {
    let mut rows: Vec<(i32, i64)> = db
        .query_all(sql, &[])
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    rows.sort_unstable();
    Ok(rows)
}

const SHELF: &str = sql!(
    "CREATE TABLE shelf (id int NOT NULL, kind int NOT NULL);
     INSERT INTO shelf VALUES (1, 10), (2, 20), (2, 20), (2, 21), (3, 30), (4, 40);"
);

// [spec:pgorm:def:macros.prql+2/test]    PRQL text's intersect and remove are
// INTERSECT ALL and EXCEPT ALL of whole rows by position: operands that each
// read a different column of a table nothing declares combine whole, and a
// stage after the operation reads its whole result
#[pgorm_macros::test]
async fn set_operations_combine_operands_read_apart() -> Result<(), Error> {
    let ctx = TestContext::new("prql_macro_set_operations").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SHELF).await?;

    let (query, _) = prql!(
        "from shelf
         filter id > 1
         intersect (from shelf | filter kind < 30)
         group {id} (aggregate {n = count this})"
    );
    assert_eq!(
        query,
        "WITH table_1 AS (SELECT * FROM shelf WHERE id > 1 \
         INTERSECT ALL SELECT * FROM shelf WHERE kind < 30) \
         SELECT id, COUNT(*) AS n FROM table_1 GROUP BY id"
    );
    assert_eq!(
        pairs(&db, query).await?,
        pairs(
            &db,
            "SELECT id, count(*) FROM (SELECT * FROM shelf WHERE id > 1 \
             INTERSECT ALL SELECT * FROM shelf WHERE kind < 30) c GROUP BY id"
        )
        .await?
    );

    let (query, _) = prql!(
        "from shelf
         filter id > 1
         remove (from shelf | filter kind < 30)
         select {id, n = kind}"
    );
    assert_eq!(
        query,
        "WITH table_1 AS (SELECT * FROM shelf WHERE id > 1 \
         EXCEPT ALL SELECT * FROM shelf WHERE kind < 30) \
         SELECT id, kind AS n FROM table_1"
    );
    let removed: Vec<(i32, i32)> = (query, Values(vec![])).into_tuple().all(&db).await?;
    let mut removed = removed;
    removed.sort_unstable();
    assert_eq!(removed, [(3, 30), (4, 40)]);

    // Each row of the other operand cancels one row: (2, 20) is on the shelf
    // twice and in the other operand once.
    let (query, _) = prql!(
        "from shelf
         remove (from shelf | filter kind == 20 | take 1)
         group {id} (aggregate {n = count this})"
    );
    assert_eq!(
        pairs(&db, query).await?,
        pairs(
            &db,
            "SELECT id, count(*) FROM (SELECT * FROM shelf \
             EXCEPT ALL (SELECT * FROM shelf WHERE kind = 20 LIMIT 1)) c GROUP BY id"
        )
        .await?
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}
