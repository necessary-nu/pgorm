#![allow(unused_imports, dead_code)]

pub mod common;

use std::num::NonZeroU64;

pub use common::{TestContext, features::*, setup::*};
use pgorm::{
    CursorPlan, CursorWindow, DatabaseRef, ErasedCursor, ErasedPages, KeyedRead, PagedRead,
    TransactionTrait,
    entity::prelude::*,
    graph_cursor,
    metric::NoOpMetrics,
    pgorm_query::IntoValueTuple,
    tests_cfg::{cake, fruit},
};
use pretty_assertions::assert_eq;

const TABLES: &str = "CREATE TABLE cake (id int PRIMARY KEY, name text NOT NULL);
     CREATE TABLE fruit (id int PRIMARY KEY, name text NOT NULL, cake_id int REFERENCES cake);
     INSERT INTO cake VALUES (1, 'a'), (2, 'b'), (3, 'c');
     INSERT INTO fruit VALUES (10, 'x', 1), (11, 'y', 1), (12, 'z', 3);";

// [spec:pgorm:def:conn.database-ref+1/test]    one executor type over a connection and a transaction
// [spec:pgorm:def:exec.paginator.erased/test]    an erased paginator's pages and count are the paginator's
// [spec:pgorm:def:exec.cursor.erased/test]    an erased graph cursor reads the typed cursor's rows under a plan
#[pgorm_macros::test]
async fn erased_reads_run_on_either_executor() -> Result<(), Error> {
    let ctx = TestContext::new("erased_read_tests").await;
    let mut db = ctx.db.get().await?;
    db.batch_execute(TABLES).await?;

    let pages: Box<dyn ErasedPages<i32>> = Box::new(PagedRead::new(
        cake::Entity::find().order_by_asc(cake::Column::Id),
        |cake: cake::Model| cake.id,
    ));
    let size = NonZeroU64::new(2).expect("two is not zero");
    assert_eq!(pages.fetch(DatabaseRef::from(&db), size, 1).await?, [3]);
    let counted = pages.count(DatabaseRef::from(&db), size).await?;
    assert_eq!((counted.number_of_items, counted.number_of_pages), (3, 2));

    let graph = cake::Entity::graph().join_maybe::<fruit::Entity>(cake::Relation::Fruit.def());
    let cursor = graph_cursor::<_, _, _, NoOpMetrics>(
        &graph,
        &[cake::Column::Name],
        |(cake, fruit): (cake::Model, Option<fruit::Model>)| (cake.id, fruit.map(|f| f.id)),
    )?;
    let plan = CursorPlan {
        after: Some(("a", 1, 10).into_value_tuple()),
        window: Some(CursorWindow::First(2)),
        ..CursorPlan::default()
    };
    let rows = db
        .transaction(async |tx| cursor.all(DatabaseRef::from(&*tx), plan.clone()).await)
        .await
        .expect("the cursor reads inside a transaction");
    assert_eq!(rows, [(1, Some(11)), (2, None)]);

    let last = CursorPlan {
        window: Some(CursorWindow::Last(1)),
        descending: true,
        ..CursorPlan::default()
    };
    assert_eq!(
        cursor.all(DatabaseRef::from(&db), last).await?,
        [(1, Some(10))]
    );
    let refused = CursorPlan {
        after: Some(("a", 1).into_value_tuple()),
        ..CursorPlan::default()
    };
    assert!(cursor.statement(&refused).is_err());

    drop(db);
    ctx.delete().await;
    Ok(())
}
