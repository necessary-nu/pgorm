//! The scratch databases the guide's examples run in, against a live server:
//! the bakery holds the rows the chapters describe, and a finished scratch
//! database is gone.

use pgorm::entity::prelude::*;
use pgorm_guide_support::{Scratch, baker, bakery, cake, cake_baker, customer, order, order_line};

/// Whether the server still holds a database named `name`.
async fn exists(scratch_url: &str, name: &str) -> bool {
    let base = scratch_url
        .strip_suffix(&format!("/{name}"))
        .expect("a scratch URL is the server's URL and the database's name");
    let maintenance = pgorm_test_support::connect(base, "postgres")
        .await
        .expect("the maintenance database accepts a connection");
    maintenance
        .query_opt("SELECT 1 FROM pg_database WHERE datname = $1", &[&name])
        .await
        .expect("pg_database is readable")
        .is_some()
}

// [spec:pgorm:req:guide.examples/test]    the bakery holds the rows the chapters describe, by id
#[tokio::test]
async fn the_bakery_holds_the_rows_its_documentation_lists() -> Result<(), Error> {
    let scratch = Scratch::bakery("support_rows").await;
    let db = scratch.pool.get().await?;

    let bakeries: Vec<(i32, String)> = bakery::Entity::find()
        .select([bakery::Column::Id, bakery::Column::Name])
        .order_by_asc(bakery::Column::Id)
        .into_tuple()
        .all(&db)
        .await?;
    assert_eq!(
        bakeries,
        [(1, "Sunrise".to_owned()), (2, "Moonlight".to_owned())]
    );

    let cakes: Vec<(i32, String, i32)> = cake::Entity::find()
        .select([cake::Column::Id, cake::Column::Name, cake::Column::BakeryId])
        .order_by_asc(cake::Column::Id)
        .into_tuple()
        .all(&db)
        .await?;
    let expected = [
        (1, "Chocolate Fudge", 1),
        (2, "Lemon Drizzle", 1),
        (3, "Carrot Cake", 2),
        (4, "Vanilla Sponge", 2),
        (5, "Chocolate Torte", 1),
    ];
    assert_eq!(cakes.len(), expected.len());
    for ((id, name, bakery_id), (want_id, want_name, want_bakery)) in cakes.iter().zip(expected) {
        assert_eq!(
            (*id, name.as_str(), *bakery_id),
            (want_id, want_name, want_bakery)
        );
    }

    let bakes: Vec<(i32, i32)> = cake_baker::Entity::find()
        .select([cake_baker::Column::BakerId, cake_baker::Column::CakeId])
        .order_by_asc(cake_baker::Column::BakerId)
        .order_by_asc(cake_baker::Column::CakeId)
        .into_tuple()
        .all(&db)
        .await?;
    assert_eq!(bakes, [(1, 1), (1, 2), (2, 2), (2, 5), (3, 3), (3, 4)]);

    assert_eq!(baker::Entity::find().count(&db).await?, 3);
    assert_eq!(customer::Entity::find().count(&db).await?, 2);
    assert_eq!(order::Entity::find().count(&db).await?, 2);
    let lines: Vec<(i32, i32, i32)> = order_line::Entity::find()
        .select([
            order_line::Column::OrderId,
            order_line::Column::CakeId,
            order_line::Column::Quantity,
        ])
        .order_by_asc(order_line::Column::Id)
        .into_tuple()
        .all(&db)
        .await?;
    assert_eq!(lines, [(1, 1, 2), (1, 2, 1), (2, 3, 1)]);

    // The next id each table hands out follows the seeded rows, so an
    // example inserting a cake gets 6.
    let next = cake::ActiveModel {
        name: set("Scone"),
        flavor: set(cake::Flavor::Lemon),
        price: set(Decimal::new(250, 2)),
        gluten_free: set(false),
        bakery_id: set(1),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    assert_eq!(next.id, 6);

    drop(db);
    let (url, name) = (scratch.url(), scratch.name().to_owned());
    assert!(exists(&url, &name).await);
    scratch.finish().await;
    assert!(!exists(&url, &name).await, "{name} outlived finish()");
    Ok(())
}

// [spec:pgorm:req:guide.examples/test]    an empty scratch database holds no table, and its names agree
#[tokio::test]
async fn an_empty_scratch_database_holds_no_table() -> Result<(), Error> {
    let scratch = Scratch::empty("support_empty").await;
    assert!(scratch.name().starts_with("guide_support_empty_"));
    assert!(scratch.url().ends_with(&format!("/{}", scratch.name())));
    assert_eq!(scratch.config().get_dbname(), Some(scratch.name()));

    let db = scratch.pool.get().await?;
    let row = db
        .query_one(
            "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public'",
            &[],
        )
        .await?;
    assert_eq!(row.get::<_, i64>(0), 0);
    drop(db);
    scratch.finish().await;
    Ok(())
}
