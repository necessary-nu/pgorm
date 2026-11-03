#![allow(unused_imports, dead_code)]

//! The disjoint entities pgorm-codegen generates from the statements
//! `Schema::create_disjoint` writes, compiled, and run against a live server.
//!
//! pgorm-codegen's own tests hold the writer's output to the files of
//! `codegen_disjoint`, so what is compiled and run here is what the generator
//! writes. The round trip closes here: the fixtures' schema, generated from,
//! is the schema the generated enums write, statement for statement, and on
//! a server it runs as written and holds the generated entities' rows to
//! their kinds.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo nextest run --test codegen_disjoint_tests`

pub mod common;
pub use common::{TestContext, setup::*};

use pgorm::entity::prelude::*;
use pgorm::{
    ConnectionTrait, Disjoint, KindedDisjoint, Schema, TransactionTrait, check_disjoint, set,
};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

mod codegen_disjoint;

use codegen_disjoint::listing::{Listing, ListingKind};
use codegen_disjoint::vehicle::{Vehicle, VehicleKind};
use codegen_disjoint::{car, flat, listing, truck, vehicle};

const SCHEMA: &str = include_str!("codegen_disjoint/schema.sql");

/// The statements `create_disjoint` returns for `D`, rendered.
fn statements<D: KindedDisjoint>() -> Vec<String> {
    Schema::new()
        .create_disjoint::<D>()
        .expect("the disjoint schema is generated")
        .iter()
        .map(ToString::to_string)
        .collect()
}

// [spec:pgorm:sem:codegen.entity.disjoint+2/test]    the round trip: the schema the vehicle and listing fixtures' `create_disjoint` writes, generated from, is the schema the generated enums' `create_disjoint` writes, statement for statement, and the generated declarations pass the run-time check
#[test]
fn generated_enums_write_the_schema_read_from() {
    let file: Vec<&str> = SCHEMA
        .split(";\n")
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .collect();
    let mut generated = statements::<Vehicle>();
    generated.push(r#"CREATE SCHEMA "market""#.to_owned());
    generated.extend(statements::<Listing>());
    assert_eq!(file, generated);

    assert_eq!(
        statements::<Vehicle>(),
        statements::<pgorm::tests_cfg::vehicle::Vehicle>()
    );
    assert_eq!(
        statements::<Listing>(),
        statements::<pgorm::tests_cfg::listing::Listing>()
    );
    check_disjoint::<Vehicle>().expect("the generated vehicle is sound");
    check_disjoint::<Listing>().expect("the generated listing is sound");
}

#[track_caller]
fn refused_by<T: std::fmt::Debug>(outcome: Result<T, Error>, constraint: &str) {
    match outcome {
        Err(Error::Postgres(error)) => {
            assert_eq!(
                error.code(),
                Some(&SqlState::FOREIGN_KEY_VIOLATION),
                "{error}"
            );
            assert_eq!(
                error.as_db_error().and_then(|db| db.constraint()),
                Some(constraint),
                "{error}"
            );
        }
        other => panic!("expected `{constraint}` to refuse the write, got {other:?}"),
    }
}

// [spec:pgorm:sem:codegen.entity.disjoint+2/test]    against a live server: the schema runs as written, the generated base and kind entities write and read their rows as ordinary entities, each kind's row reaching its base row and the base row its kind's through the generated relations, and the constraints the generated enum stands for still refuse a car with no row of its own
#[pgorm_macros::test]
async fn generated_entities_write_the_schema_live() -> Result<(), Error> {
    let ctx = TestContext::new("codegen_disjoint").await;
    let mut db = ctx.db.get().await?;
    db.batch_execute(SCHEMA).await?;

    let (sedan, kit) = db
        .transaction(async |txn| {
            txn.execute("SET CONSTRAINTS ALL DEFERRED", &[]).await?;
            let sedan = vehicle::ActiveModel {
                kind: set(VehicleKind::Car),
                name: set("sedan"),
                ..Default::default()
            }
            .insert(&txn)
            .await?;
            let kit = car::ActiveModel {
                id: set(sedan.id),
                seats: set(5i16),
            }
            .insert(&txn)
            .await?;
            Ok::<_, Error>((sedan, kit))
        })
        .await
        .expect("a car and its row commit together");
    let bike = vehicle::ActiveModel {
        kind: set(VehicleKind::Bike),
        name: set("pushbike"),
        ..Default::default()
    }
    .insert(&db)
    .await?;

    assert_eq!(
        vehicle::Entity::find().all(&db).await?,
        [sedan.clone(), bike.clone()]
    );
    assert_eq!(sedan.find_related(car::Entity).one(&db).await?, kit);
    assert_eq!(kit.find_related(vehicle::Entity).one(&db).await?, sedan);
    assert_eq!(bike.find_related(truck::Entity).one_opt(&db).await?, None);

    refused_by(
        vehicle::ActiveModel {
            kind: set(VehicleKind::Car),
            name: set("ghost"),
            ..Default::default()
        }
        .insert(&db)
        .await,
        "fk-vehicle-car_id",
    );

    let flat = db
        .transaction(async |txn| {
            txn.execute("SET CONSTRAINTS ALL DEFERRED", &[]).await?;
            listing::ActiveModel {
                tenant_id: set(7),
                id: set(1),
                category: set(ListingKind::Apartment),
                title: set("loft"),
            }
            .insert(&txn)
            .await?;
            let flat = flat::ActiveModel {
                tenant: set(7),
                listing: set(1),
                rooms: set(3i16),
            }
            .insert(&txn)
            .await?;
            Ok::<_, Error>(flat)
        })
        .await
        .expect("an apartment and its flat commit together");
    let loft = flat.find_related(listing::Entity).one(&db).await?;
    assert_eq!(
        (loft.tenant_id, loft.id, loft.category),
        (7, 1, ListingKind::Apartment)
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
