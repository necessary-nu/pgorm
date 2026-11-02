#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, bakery_chain::*, setup::*};
use futures::TryStreamExt;
use pgorm::{PartialModel, entity::prelude::*};
use pretty_assertions::assert_eq;

#[derive(Debug, PartialEq, FromQueryResult, PartialModel)]
#[pgorm(entity = "Bakery")]
struct BakeryName {
    name: String,
}

#[pgorm_macros::test]
async fn stream_tests() -> Result<(), Error> {
    let ctx = TestContext::new("stream_tests").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    seed(&db).await?;
    stream_many(&db).await?;
    stream_partial(&db).await?;
    stream_empty(&db).await?;
    stream_in_transaction(&mut db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

async fn seed(db: &DatabaseConnection) -> Result<(), Error> {
    for (name, margin) in [
        ("SeaSide Bakery", 10.4),
        ("Top Bakery", 15.0),
        ("Third Bakery", 20.5),
        ("Fourth Bakery", 5.25),
    ] {
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

// [spec:pgorm:sem:exec.stream.decode+1/test]    ordered multi-row stream matches `all`
async fn stream_many(db: &DatabaseConnection) -> Result<(), Error> {
    let expected = Bakery::find()
        .order_by_asc(bakery::Column::Id)
        .all(db)
        .await?;
    assert_eq!(expected.len(), 4);

    let streamed: Vec<bakery::Model> = Bakery::find()
        .order_by_asc(bakery::Column::Id)
        .stream(db)
        .await?
        .try_collect()
        .await?;

    assert_eq!(streamed, expected);

    Ok(())
}

// [spec:pgorm:def:exec.stream+2/test]    partial models decode from a stream
async fn stream_partial(db: &DatabaseConnection) -> Result<(), Error> {
    let streamed: Vec<BakeryName> = Bakery::find()
        .order_by_asc(bakery::Column::Id)
        .stream_partial_model::<_, BakeryName>(db)
        .await?
        .try_collect()
        .await?;

    assert_eq!(
        streamed,
        [
            BakeryName {
                name: "SeaSide Bakery".to_owned()
            },
            BakeryName {
                name: "Top Bakery".to_owned()
            },
            BakeryName {
                name: "Third Bakery".to_owned()
            },
            BakeryName {
                name: "Fourth Bakery".to_owned()
            },
        ]
    );

    Ok(())
}

// [spec:pgorm:sem:exec.stream.decode+1/test]    zero rows terminates immediately
async fn stream_empty(db: &DatabaseConnection) -> Result<(), Error> {
    let streamed: Vec<bakery::Model> = Bakery::find()
        .filter(bakery::Column::Name.eq("No Such Bakery"))
        .stream(db)
        .await?
        .try_collect()
        .await?;

    assert!(streamed.is_empty());

    Ok(())
}

// [spec:pgorm:def:exec.stream+2/test]    streaming through a transaction
async fn stream_in_transaction(db: &mut DatabaseConnection) -> Result<(), Error> {
    db.transaction(async |txn| {
        bakery::ActiveModel {
            name: set("Transient Bakery"),
            profit_margin: set(1.5),
            ..Default::default()
        }
        .insert(&txn)
        .await?;

        let streamed: Vec<bakery::Model> = Bakery::find()
            .order_by_asc(bakery::Column::Id)
            .stream(&txn)
            .await?
            .try_collect()
            .await?;

        assert_eq!(streamed.len(), 5);
        assert_eq!(streamed[4].name, "Transient Bakery");

        Ok::<_, Error>(())
    })
    .await
    .expect("the transaction commits");

    let after = Bakery::find().all(db).await?;
    assert_eq!(after.len(), 5);

    Ok(())
}
