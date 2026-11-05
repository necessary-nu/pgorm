#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, bakery_chain::*, setup::*};
pub use pgorm::entity::*;
pub use pgorm::{ConnectionTrait, Error, QueryFilter, QueryOrder, QuerySelect, set};

// Run the test locally:
// DATABASE_URL=postgres://postgres:postgres@127.0.0.1:54329 cargo test --test query_tests
// [spec:pgorm:sem:exec.crud.select+4/test]    `one` fails with RecordNotFound on
// zero rows where `one_opt` reports `None` — pgorm's deliberate difference
#[pgorm_macros::test]
pub async fn find_one_with_no_result() {
    let ctx = TestContext::new("find_one_with_no_result").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();

    let bakery = Bakery::find().one_opt(&db).await.unwrap();
    assert_eq!(bakery, None);

    assert!(matches!(
        Bakery::find().one(&db).await,
        Err(Error::RecordNotFound)
    ));

    drop(db);
    ctx.delete().await;
}

#[pgorm_macros::test]
pub async fn find_one_with_result() {
    let ctx = TestContext::new("find_one_with_result").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();

    let bakery = bakery::ActiveModel {
        name: set("SeaSide Bakery"),
        profit_margin: set(10.4),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect("could not insert bakery");

    let result = Bakery::find().one(&db).await.unwrap();

    assert_eq!(result.id, bakery.id);

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:sem:exec.crud.select+4/test]    the same split on a filtered select
#[pgorm_macros::test]
pub async fn find_by_id_with_no_result() {
    let ctx = TestContext::new("find_by_id_with_no_result").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();

    let bakery = Bakery::find_by_id(999).one_opt(&db).await.unwrap();
    assert_eq!(bakery, None);

    assert!(matches!(
        Bakery::find_by_id(999).one(&db).await,
        Err(Error::RecordNotFound)
    ));

    drop(db);
    ctx.delete().await;
}

#[pgorm_macros::test]
pub async fn find_by_id_with_result() {
    let ctx = TestContext::new("find_by_id_with_result").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();

    let bakery = bakery::ActiveModel {
        name: set("SeaSide Bakery"),
        profit_margin: set(10.4),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect("could not insert bakery");

    let result = Bakery::find_by_id(bakery.id).one(&db).await.unwrap();

    assert_eq!(result.id, bakery.id);

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:sem:exec.crud.select+4/test]    `one` and `one_opt` narrow a row limit to one row
// and never widen it: a limit of zero, plain or tied, reads no row
// [spec:pgorm:req:exec.tie-limit+1/test]
#[pgorm_macros::test]
pub async fn one_never_widens_a_row_limit() -> Result<(), Error> {
    use pgorm::QueryTrait;

    let ctx = TestContext::new("one_never_widens_a_row_limit").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    for name in ["SeaSide Bakery", "Top Bakery"] {
        bakery::ActiveModel {
            name: set(name),
            profit_margin: set(10.0),
            ..Default::default()
        }
        .insert(&db)
        .await?;
    }
    let by_margin = || Bakery::find().order_by_asc(bakery::Column::ProfitMargin);
    let tied = |count| {
        let mut query = by_margin();
        QueryTrait::query(&mut query).limit_with_ties(count);
        query
    };

    assert_eq!(by_margin().limit(0).one_opt(&db).await?, None);
    assert!(matches!(
        by_margin().limit(0).one(&db).await,
        Err(Error::RecordNotFound)
    ));
    assert_eq!(tied(0).one_opt(&db).await?, None);
    assert!(matches!(tied(0).one(&db).await, Err(Error::RecordNotFound)));

    // Both rows tie on the margin, so the tie limit alone returns two, which
    // `query_opt` would refuse: narrowed, one row is read.
    assert_eq!(tied(1).all(&db).await?.len(), 2);
    assert!(tied(1).one_opt(&db).await?.is_some());
    assert_eq!(by_margin().limit(5).one(&db).await?.profit_margin, 10.0);

    drop(db);
    ctx.delete().await;
    Ok(())
}

#[pgorm_macros::test]
pub async fn find_all_with_no_result() {
    let ctx = TestContext::new("find_all_with_no_result").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();

    let bakeries = Bakery::find().all(&db).await.unwrap();
    assert_eq!(bakeries.len(), 0);

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:sem:exec.crud.select+4/test]    `all` decodes every returned row
#[pgorm_macros::test]
pub async fn find_all_with_result() {
    let ctx = TestContext::new("find_all_with_result").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();

    let _ = bakery::ActiveModel {
        name: set("SeaSide Bakery"),
        profit_margin: set(10.4),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect("could not insert bakery");

    let _ = bakery::ActiveModel {
        name: set("Top Bakery"),
        profit_margin: set(15.0),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect("could not insert bakery");

    let bakeries = Bakery::find().all(&db).await.unwrap();

    assert_eq!(bakeries.len(), 2);

    drop(db);
    ctx.delete().await;
}

#[pgorm_macros::test]
pub async fn find_all_filter_no_result() {
    let ctx = TestContext::new("find_all_filter_no_result").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();

    let _ = bakery::ActiveModel {
        name: set("SeaSide Bakery"),
        profit_margin: set(10.4),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect("could not insert bakery");

    let _ = bakery::ActiveModel {
        name: set("Top Bakery"),
        profit_margin: set(15.0),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect("could not insert bakery");

    let bakeries = Bakery::find()
        .filter(bakery::Column::Name.contains("Good"))
        .all(&db)
        .await
        .unwrap();

    assert_eq!(bakeries.len(), 0);

    drop(db);
    ctx.delete().await;
}

#[pgorm_macros::test]
pub async fn find_all_filter_with_results() {
    let ctx = TestContext::new("find_all_filter_with_results").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();

    let _ = bakery::ActiveModel {
        name: set("SeaSide Bakery"),
        profit_margin: set(10.4),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect("could not insert bakery");

    let _ = bakery::ActiveModel {
        name: set("Top Bakery"),
        profit_margin: set(15.0),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect("could not insert bakery");

    let bakeries = Bakery::find()
        .filter(bakery::Column::Name.contains("Bakery"))
        .all(&db)
        .await
        .unwrap();

    assert_eq!(bakeries.len(), 2);

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:sem:sql.render.empty-in+1/test]    against a live server: an empty
// `IN` selects nothing, an empty `NOT IN` selects everything
// [spec:pgorm:req:sql.ast.expr.in+1/test]
#[pgorm_macros::test]
pub async fn empty_in_and_not_in_filter_asymmetry() {
    let ctx = TestContext::new("empty_in_and_not_in_filter_asymmetry").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();

    for name in ["SeaSide Bakery", "Top Bakery"] {
        bakery::ActiveModel {
            name: set(name),
            profit_margin: set(10.4),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("could not insert bakery");
    }

    let none = Bakery::find()
        .filter(bakery::Column::Id.is_in(Vec::<i32>::new()))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(none.len(), 0);

    let all = Bakery::find()
        .filter(bakery::Column::Id.is_not_in(Vec::<i32>::new()))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:def:sql.render.value-literals+5/test]    against a live server: an
// inline char literal parses and comes back as the char it was rendered from
#[pgorm_macros::test]
pub async fn char_literal_round_trips_through_server() {
    use pgorm::pgorm_query::{Query, QueryBuilder, Value};

    let ctx = TestContext::new("char_literal_round_trips_through_server").await;
    let db = ctx.db.get().await.unwrap();

    for character in ['a', 'é', '—', '\''] {
        let sql = Query::select()
            .expr(Value::Char(Some(character)))
            .to_string();
        let row = db.query_one(sql.as_str(), &[]).await.unwrap();
        let echoed: String = row.get(0);
        assert_eq!(echoed, character.to_string());
    }

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:sem:exec.crud.select+4/test]    `all` aborts on the first decode
// error instead of yielding a partial result set
#[pgorm_macros::test]
pub async fn select_only_exclude_option_fields() {
    let ctx = TestContext::new("select_only_exclude_option_fields").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();

    let _ = customer::ActiveModel {
        name: set("Alice"),
        notes: "Want to communicate with Bob".into(),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect("could not insert customer");

    let _ = customer::ActiveModel {
        name: set("Bob"),
        notes: "Just listening".into(),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect("could not insert customer");

    // An absent column is not a NULL value: only `WasNull` decodes to `None`,
    // every other decode error propagates. A custom projection has to name the
    // model it decodes into, so the mismatch is a decode error, not a silent
    // `Vec<customer::Model>`.
    let err = Customer::find()
        .select([customer::Column::Id, customer::Column::Name])
        .into_model::<customer::Model>()
        .all(&db)
        .await
        .expect_err("an absent `notes` column must not decode as None");

    assert!(matches!(err, Error::Postgres(_)), "unexpected error: {err}");

    let customers = Customer::find()
        .select([
            customer::Column::Id,
            customer::Column::Name,
            customer::Column::Notes,
        ])
        .into_model::<customer::Model>()
        .all(&db)
        .await
        .unwrap();

    assert_eq!(customers.len(), 2);
    assert_eq!(
        customers[0].notes,
        Some("Want to communicate with Bob".to_owned())
    );
    assert_eq!(customers[1].notes, Some("Just listening".to_owned()));

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:sem:exec.crud.select+4/test]    `SelectorRaw::one` / `one_opt`
// execute the statement exactly as written, with no `LIMIT` injected
// [spec:pgorm:def:exec.crud+2/test]    `Select::from_raw_sql` and
// `SelectorRaw::into_model` re-targeting the decoded type
#[pgorm_macros::test]
pub async fn raw_selector_one_semantics() -> Result<(), Error> {
    use pgorm::FromQueryResult;
    use pgorm_query::{Value, Values};

    let ctx = TestContext::new("raw_selector_one_semantics").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    for (name, margin) in [("SeaSide Bakery", 10.4), ("Top Bakery", 15.0)] {
        bakery::ActiveModel {
            name: set(name),
            profit_margin: set(margin),
            ..Default::default()
        }
        .insert(&db)
        .await?;
    }

    const ALL: &str = r#"SELECT "id", "name", "profit_margin" FROM "bakery" ORDER BY "id" ASC"#;

    // Zero rows: `one` fails, `one_opt` reports `None`.
    let missing = || {
        Bakery::find().from_raw_sql(
            format!(r#"{ALL} OFFSET $1"#),
            Values(vec![Value::BigInt(Some(99))]),
        )
    };
    assert!(matches!(
        missing().one(&db).await,
        Err(Error::RecordNotFound)
    ));
    assert_eq!(missing().one_opt(&db).await?, None);

    // No `LIMIT` is injected, so a raw statement matching two rows reaches
    // `query_opt` with both of them and the driver rejects the row count...
    let unlimited = Bakery::find()
        .from_raw_sql(ALL.to_owned(), Values(Vec::new()))
        .one(&db)
        .await;
    match unlimited {
        Err(Error::Postgres(err)) => assert_eq!(
            err.to_string(),
            "query returned an unexpected number of rows"
        ),
        other => panic!("unexpected result: {other:?}"),
    }

    // ... whereas `Selector::one` sets `LIMIT 1` on the same select first, so
    // the identical query returns the first row.
    let limited = Bakery::find()
        .order_by_asc(bakery::Column::Id)
        .one(&db)
        .await?;
    assert_eq!(limited.name, "SeaSide Bakery");

    // A raw statement is otherwise run exactly as written.
    assert_eq!(
        Bakery::find()
            .from_raw_sql(format!("{ALL} LIMIT 1"), Values(Vec::new()))
            .one(&db)
            .await?
            .name,
        "SeaSide Bakery"
    );
    assert_eq!(
        Bakery::find()
            .from_raw_sql(format!("{ALL} LIMIT 2"), Values(Vec::new()))
            .all(&db)
            .await?
            .len(),
        2
    );

    // `SelectorRaw::into_model` re-targets the decoded type.
    #[derive(Debug, PartialEq, FromQueryResult)]
    struct BakeryName {
        name: String,
    }

    assert_eq!(
        Bakery::find()
            .from_raw_sql(format!("{ALL} LIMIT 1"), Values(Vec::new()))
            .into_model::<BakeryName>()
            .one(&db)
            .await?,
        BakeryName {
            name: "SeaSide Bakery".to_owned()
        }
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:sql.render.select-order+7/test]    a named window combined with ORDER BY and
// LIMIT, run against a live server
// [spec:pgorm:req:sql.render.window+7/test]
#[pgorm_macros::test]
pub async fn named_window_over_a_real_query() -> Result<(), Error> {
    use pgorm::alias;
    use pgorm::pgorm_query::{Expr, Func, Order, Query, QueryBuilder, WindowStatement};

    let ctx = TestContext::new("named_window_over_a_real_query").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    for (name, profit_margin) in [("A", 1.0), ("B", 1.0), ("C", 2.0), ("D", 2.0), ("E", 2.0)] {
        bakery::ActiveModel {
            name: set(name),
            profit_margin: set(profit_margin),
            ..Default::default()
        }
        .insert(&db)
        .await?;
    }

    let margin = alias("margin");
    let sql = Query::select()
        .column(bakery::Column::Name)
        .expr_window_name(Func::count(Expr::col(bakery::Column::Id)), margin)
        .from(bakery::Entity)
        .window(
            margin,
            WindowStatement::partition_by(bakery::Column::ProfitMargin),
        )
        .order_by(bakery::Column::Name, Order::Asc)
        .limit(4)
        .to_string();

    // The window is counted over the whole partition, so LIMIT cannot reach it.
    let rows = db.query_all(&sql, &[]).await?;
    let peers: Vec<(String, i64)> = rows.iter().map(|row| (row.get(0), row.get(1))).collect();

    pretty_assertions::assert_eq!(
        peers,
        [
            ("A".to_owned(), 2),
            ("B".to_owned(), 2),
            ("C".to_owned(), 3),
            ("D".to_owned(), 3),
        ]
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}
