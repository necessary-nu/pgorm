#![allow(unused_imports, dead_code)]

//! The changes `Schema` writes to a disjoint entity's kinds once its tables
//! exist: their statements and steps, the changes they refuse, and, run on a
//! live server, the entity reading and writing as its new declaration says
//! after each.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo nextest run --test disjoint_evolve_tests`

pub mod common;
pub use common::{TestContext, setup::*};

use pgorm::{
    ConnectionTrait, DatabaseConnection, DdlStatement, DisjointChange, DisjointShape, DisjointStep,
    Error, KindedDisjoint, Schema, TransactionError, TransactionTrait, check_disjoint,
    entity::prelude::*,
    pgorm_query::{Name, TableName},
};
use pretty_assertions::assert_eq;
use std::fmt::Debug;
use tokio_postgres::error::SqlState;

pub mod car {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "car")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub seats: i16,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

pub mod van {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "van")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub doors: i16,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

pub mod bike {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "bike")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub gears: i16,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// The base of one declaration of the fleet: an entity over `$table`, whose
/// kind column is of `$kind`. Each version of the declaration a change starts
/// from or ends at has its own, as a program's entity code has before and
/// after it changes. The enums are written out beside them, since the
/// `Disjoint` derive does not take its variants through a `macro_rules!`.
macro_rules! fleet_base {
    ($base:ident, $table:literal, $kind:ident) => {
        pub mod $base {
            use pgorm::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = $table)]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
                pub kind: super::$kind,
                pub name: String,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }
    };
}

fleet_base!(fleet_one, "fleet", FleetOneKind);

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = fleet_one::Entity, enum_name = "fleet_kind")]
pub enum FleetOne {
    Car(car::Model),
    Bike,
}

fleet_base!(fleet_van, "fleet", FleetVanKind);

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = fleet_van::Entity, enum_name = "fleet_kind")]
pub enum FleetVan {
    Car(car::Model),
    Van(van::Model),
    Bike,
}

fleet_base!(fleet_scooter, "fleet", FleetScooterKind);

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = fleet_scooter::Entity, enum_name = "fleet_kind")]
pub enum FleetScooter {
    Car(car::Model),
    Bike,
    Scooter,
}

fleet_base!(fleet_tram, "fleet", FleetTramKind);

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = fleet_tram::Entity, enum_name = "fleet_kind")]
pub enum FleetTram {
    Tram,
    Car(car::Model),
    Bike,
}

fleet_base!(fleet_bike, "fleet", FleetBikeKind);

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = fleet_bike::Entity, enum_name = "fleet_kind")]
pub enum FleetBike {
    Car(car::Model),
    Bike(bike::Model),
}

fleet_base!(yard_one, "yard", YardOneKind);

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = yard_one::Entity, enum_name = "Yard Kind", schema_name = "Fleet Yard")]
pub enum YardOne {
    Car(car::Model),
    Bike,
}

fleet_base!(yard_van, "yard", YardVanKind);

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = yard_van::Entity, enum_name = "Yard Kind", schema_name = "Fleet Yard")]
pub enum YardVan {
    Car(car::Model),
    Van(van::Model),
    Bike,
}

fleet_base!(yard_van_unit, "yard", YardVanUnitKind);

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = yard_van_unit::Entity, enum_name = "Yard Kind", schema_name = "Fleet Yard")]
pub enum YardVanUnit {
    Car(car::Model),
    Van,
    Bike,
}

/// The base of a fleet whose kind column defaults to `bike`, before and after
/// a van joins it.
pub mod kept_one {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "fleet")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(default_value = "bike")]
        pub kind: super::KeptOneKind,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = kept_one::Entity, enum_name = "fleet_kind")]
pub enum KeptOne {
    Car(car::Model),
    Bike,
}

pub mod kept_van {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "fleet")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(default_value = "bike")]
        pub kind: super::KeptVanKind,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = kept_van::Entity, enum_name = "fleet_kind")]
pub enum KeptVan {
    Car(car::Model),
    Van(van::Model),
    Bike,
}

/// A base whose kind column is nullable, which `check_disjoint` refuses.
pub mod loose_fleet {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "fleet")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub kind: Option<super::LooseFleetKind>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = loose_fleet::Entity, enum_name = "fleet_kind")]
pub enum LooseFleet {
    Car(car::Model),
    Bike(bike::Model),
}

/// Each step's statements, rendered.
fn rendered(change: &DisjointChange) -> Vec<Vec<String>> {
    change
        .steps()
        .iter()
        .map(|step| step.statements().iter().map(ToString::to_string).collect())
        .collect()
}

fn refusal<T: Debug>(outcome: Result<T, Error>) -> String {
    match outcome {
        Err(Error::Type(message)) => message,
        other => panic!("expected the change refused as a type error, got {other:?}"),
    }
}

/// Run `statements` in order in one transaction on a connection of `pool`,
/// then clear the pool's statement caches, as pgorm-migration does after each
/// commit: [`values`], prepared against the kind type a change drops, fails
/// once it has (`XX000`, a cache lookup of the dropped type).
async fn run_all(pool: &DatabasePool, statements: &[DdlStatement]) -> Result<(), Error> {
    let mut db = pool.get().await?;
    let outcome = db
        .transaction(async |tx| {
            for statement in statements {
                tx.execute(&statement.to_string(), &[]).await?;
            }
            Ok::<_, Error>(())
        })
        .await;
    drop(db);
    pool.clear_statement_caches();
    match outcome {
        Ok(()) => Ok(()),
        Err(TransactionError::Transaction(error) | TransactionError::Connection(error)) => {
            Err(error)
        }
        Err(other) => Err(Error::Custom(other.to_string())),
    }
}

/// Run one step of a change, in one transaction.
async fn run(pool: &DatabasePool, step: &DisjointStep) -> Result<(), Error> {
    run_all(pool, step.statements()).await
}

/// Run every step of a change in order, each in its own transaction.
async fn run_change(pool: &DatabasePool, change: &DisjointChange) -> Result<(), Error> {
    for step in change.steps() {
        run(pool, step).await?;
    }
    Ok(())
}

/// Create `D`'s type and tables.
async fn create<D: KindedDisjoint>(pool: &DatabasePool) -> Result<(), Error> {
    run_all(pool, &Schema::new().create_disjoint::<D>()?).await
}

async fn values(db: &DatabaseConnection, type_name: &str) -> Result<Vec<String>, Error> {
    Ok(db
        .query_one(
            &format!("SELECT enum_range(NULL::{type_name})::text[]"),
            &[],
        )
        .await?
        .get(0))
}

async fn columns(db: &DatabaseConnection, table: &str) -> Result<Vec<String>, Error> {
    Ok(db
        .query_all(
            "SELECT attname::text FROM pg_attribute WHERE attrelid = $1::text::regclass \
             AND attnum > 0 AND NOT attisdropped ORDER BY attnum",
            &[&table],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect())
}

/// The SQLSTATE and constraint a refusal names.
fn refused(error: &Error) -> (Option<SqlState>, Option<String>) {
    match error {
        Error::Postgres(error) => (
            error.code().cloned(),
            error
                .as_db_error()
                .and_then(|db| db.constraint())
                .map(str::to_owned),
        ),
        _ => (None, None),
    }
}

/// Write one fleet row of each kind `FleetOne` declares, a car with its row
/// and a bike.
async fn seed_fleet_one(db: &DatabaseConnection) -> Result<(i32, i32), Error> {
    let (car, _) = FleetOne::insert(
        fleet_one::ActiveModel {
            name: set("sedan"),
            ..Default::default()
        },
        FleetOneActiveModel::Car(car::ActiveModel {
            seats: set(5_i16),
            ..Default::default()
        }),
    )?
    .exec_returning_model(db)
    .await?;
    let (bike, _) = FleetOne::insert(
        fleet_one::ActiveModel {
            name: set("tourer"),
            ..Default::default()
        },
        FleetOneActiveModel::Bike,
    )?
    .exec_returning_model(db)
    .await?;
    Ok((car.id, bike.id))
}

// [spec:pgorm:sem:disjoint.evolve+1/test]    a kind with a table is added in two steps, the value placed after the kind before it, then the table as `create_disjoint` writes it and the base's generated key columns and foreign key for it; run in one transaction the second is refused, run in two the entity reads its old rows and writes the new kind, and its constraints hold the new kind's rows
#[pgorm_macros::test]
async fn kind_with_table_added_in_two_steps_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_evolve_add").await;
    create::<FleetOne>(&ctx.db).await?;
    let db = ctx.db.get().await?;
    let (car, bike) = seed_fleet_one(&db).await?;

    let change = Schema::new().add_disjoint_kind::<FleetVan>(FleetVanKind::Van)?;
    assert_eq!(change.steps().len(), 2);
    assert!(change.steps().iter().all(|step| step.gap().is_none()));

    let together: Vec<DdlStatement> = change
        .steps()
        .iter()
        .flat_map(|step| step.statements().iter().cloned())
        .collect();
    let error = run_all(&ctx.db, &together)
        .await
        .expect_err("an added value is used before its transaction commits");
    assert_eq!(
        refused(&error).0,
        Some(SqlState::UNSAFE_NEW_ENUM_VALUE_USAGE),
        "{error}"
    );
    assert_eq!(values(&db, "fleet_kind").await?, ["car", "bike"]);

    run_change(&ctx.db, &change).await?;
    let db = ctx.db.get().await?;
    assert_eq!(values(&db, "fleet_kind").await?, ["car", "van", "bike"]);
    assert_eq!(
        columns(&db, "fleet").await?,
        ["id", "kind", "name", "car_id", "van_id"]
    );

    let (van, written) = FleetVan::insert(
        fleet_van::ActiveModel {
            name: set("camper"),
            ..Default::default()
        },
        FleetVanActiveModel::Van(van::ActiveModel {
            doors: set(4_i16),
            ..Default::default()
        }),
    )?
    .exec_returning_model(&db)
    .await?;
    assert_eq!(
        written,
        FleetVan::Van(van::Model {
            id: van.id,
            doors: 4
        })
    );
    let read: Vec<FleetVan> = FleetVan::find()
        .order_by_asc(fleet_van::Column::Id)
        .all(&db)
        .await?
        .into_iter()
        .map(|(_, value)| value)
        .collect();
    assert_eq!(
        read,
        [
            FleetVan::Car(car::Model { id: car, seats: 5 }),
            FleetVan::Bike,
            FleetVan::Van(van::Model {
                id: van.id,
                doors: 4
            }),
        ]
    );
    let _ = bike;

    let alone = db
        .execute(
            "INSERT INTO fleet (kind, name) VALUES ('van', 'ghost')",
            &[],
        )
        .await
        .expect_err("a van with no row in its table is refused");
    assert_eq!(
        refused(&alone),
        (
            Some(SqlState::FOREIGN_KEY_VIOLATION),
            Some("fk-fleet-van_id".to_owned())
        )
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.evolve+1/test]    a kind without a table is added in one step, its value placed after the last kind, and is written and read as its unit variant
#[pgorm_macros::test]
async fn unit_kind_added_in_one_step_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_evolve_add_unit").await;
    create::<FleetOne>(&ctx.db).await?;
    let db = ctx.db.get().await?;
    seed_fleet_one(&db).await?;

    let change = Schema::new().add_disjoint_kind::<FleetScooter>(FleetScooterKind::Scooter)?;
    assert_eq!(
        rendered(&change),
        [[r#"ALTER TYPE "fleet_kind" ADD VALUE 'scooter' AFTER 'bike'"#]]
    );
    run_change(&ctx.db, &change).await?;
    let db = ctx.db.get().await?;
    assert_eq!(values(&db, "fleet_kind").await?, ["car", "bike", "scooter"]);

    FleetScooter::insert(
        fleet_scooter::ActiveModel {
            name: set("kick"),
            ..Default::default()
        },
        FleetScooterActiveModel::Scooter,
    )?
    .exec_returning_model(&db)
    .await?;
    let kinds: Vec<FleetScooter> = FleetScooter::find()
        .order_by_asc(fleet_scooter::Column::Id)
        .all(&db)
        .await?
        .into_iter()
        .map(|(_, value)| value)
        .collect();
    assert_eq!(kinds.len(), 3);
    assert_eq!(kinds[2], FleetScooter::Scooter);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.evolve+1/test]    the recreating route adds a kind with a table in one step: the type made again with every value in declaration order under its own name, the rows written before keeping their kinds, the base laid out as `create_disjoint` lays it out, and the entity reading and writing every kind with its constraints
#[pgorm_macros::test]
async fn a_kind_is_added_by_recreating_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_evolve_recreate").await;
    create::<FleetOne>(&ctx.db).await?;
    let db = ctx.db.get().await?;
    let (car, bike) = seed_fleet_one(&db).await?;

    let change = Schema::new().add_disjoint_kind_recreating::<FleetVan>(FleetVanKind::Van)?;
    assert_eq!(change.steps().len(), 1);
    run_change(&ctx.db, &change).await?;
    let db = ctx.db.get().await?;
    assert_eq!(values(&db, "fleet_kind").await?, ["car", "van", "bike"]);
    assert_eq!(
        columns(&db, "fleet").await?,
        ["id", "kind", "name", "car_id", "van_id"]
    );
    let leftover: bool = db
        .query_one("SELECT to_regtype('fleet_kind_next') IS NOT NULL", &[])
        .await?
        .get(0);
    assert!(!leftover);

    let (van, _) = FleetVan::insert(
        fleet_van::ActiveModel {
            name: set("camper"),
            ..Default::default()
        },
        FleetVanActiveModel::Van(van::ActiveModel {
            doors: set(4_i16),
            ..Default::default()
        }),
    )?
    .exec_returning_model(&db)
    .await?;
    let read: Vec<FleetVan> = FleetVan::find()
        .order_by_asc(fleet_van::Column::Id)
        .all(&db)
        .await?
        .into_iter()
        .map(|(_, value)| value)
        .collect();
    assert_eq!(
        read,
        [
            FleetVan::Car(car::Model { id: car, seats: 5 }),
            FleetVan::Bike,
            FleetVan::Van(van::Model {
                id: van.id,
                doors: 4
            }),
        ]
    );

    for (statement, constraint) in [
        (
            "INSERT INTO fleet (kind, name) VALUES ('car', 'ghost')",
            "fk-fleet-car_id",
        ),
        (
            "INSERT INTO fleet (kind, name) VALUES ('van', 'ghost')",
            "fk-fleet-van_id",
        ),
    ] {
        let error = db
            .execute(statement, &[])
            .await
            .expect_err("a base row without its kind's row is refused");
        assert_eq!(
            refused(&error),
            (
                Some(SqlState::FOREIGN_KEY_VIOLATION),
                Some(constraint.to_owned())
            )
        );
    }
    let error = db
        .execute(
            &format!("INSERT INTO car (id, seats) VALUES ({bike}, 2)"),
            &[],
        )
        .await
        .expect_err("a car row for a bike is refused");
    assert_eq!(
        refused(&error),
        (
            Some(SqlState::FOREIGN_KEY_VIOLATION),
            Some("fk-car-id-kind".to_owned())
        )
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.evolve+1/test]    removing a value fails at its first statement while a base row still has the kind, changing nothing, and once none does recreates the type without it, the entity reading and writing its remaining kinds
#[pgorm_macros::test]
async fn kind_removed_once_no_row_has_it_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_evolve_remove").await;
    create::<FleetScooter>(&ctx.db).await?;
    let db = ctx.db.get().await?;
    FleetScooter::insert(
        fleet_scooter::ActiveModel {
            name: set("kick"),
            ..Default::default()
        },
        FleetScooterActiveModel::Scooter,
    )?
    .exec_returning_model(&db)
    .await?;
    FleetScooter::insert(
        fleet_scooter::ActiveModel {
            name: set("sedan"),
            ..Default::default()
        },
        FleetScooterActiveModel::Car(car::ActiveModel {
            seats: set(5_i16),
            ..Default::default()
        }),
    )?
    .exec_returning_model(&db)
    .await?;

    let change = Schema::new().remove_disjoint_kind::<FleetOne>("scooter")?;
    assert_eq!(change.steps().len(), 1);
    let error = run_change(&ctx.db, &change)
        .await
        .expect_err("a value a row still has is not removed");
    assert_eq!(
        refused(&error),
        (
            Some(SqlState::CHECK_VIOLATION),
            Some("ck-fleet-kind-scooter".to_owned())
        )
    );
    assert_eq!(values(&db, "fleet_kind").await?, ["car", "bike", "scooter"]);

    db.execute("DELETE FROM fleet WHERE kind = 'scooter'", &[])
        .await?;
    run_change(&ctx.db, &change).await?;
    let db = ctx.db.get().await?;
    assert_eq!(values(&db, "fleet_kind").await?, ["car", "bike"]);
    assert_eq!(
        columns(&db, "fleet").await?,
        ["id", "kind", "name", "car_id"]
    );

    let (bike, _) = FleetOne::insert(
        fleet_one::ActiveModel {
            name: set("tourer"),
            ..Default::default()
        },
        FleetOneActiveModel::Bike,
    )?
    .exec_returning_model(&db)
    .await?;
    let read: Vec<(i32, FleetOne)> = FleetOne::find()
        .order_by_asc(fleet_one::Column::Id)
        .all(&db)
        .await?
        .into_iter()
        .map(|(base, value)| (base.id, value))
        .collect();
    assert_eq!(read.len(), 2);
    assert!(matches!(read[0].1, FleetOne::Car(_)));
    assert_eq!(read[1], (bike.id, FleetOne::Bike));

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.evolve+1/test]    a unit kind gains its table in two steps with a gap: the table and the base's generated key columns with a foreign key added NOT VALID, which still refuses a new base row of the kind without its row; VALIDATE refuses while a row is missing and passes once the program has written them, the entity then reading the kind's rows
#[pgorm_macros::test]
async fn a_unit_kind_gains_a_table_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_evolve_add_table").await;
    create::<FleetOne>(&ctx.db).await?;
    let db = ctx.db.get().await?;
    let (_, first) = seed_fleet_one(&db).await?;
    let (_, second) = seed_fleet_one(&db).await?;

    let change = Schema::new().add_disjoint_kind_table::<FleetBike>(FleetBikeKind::Bike)?;
    let [added, validate] = change.steps() else {
        panic!("two steps, got {:?}", change.steps());
    };
    let gap = added.gap().expect("the program writes the bikes' rows");
    assert_eq!(gap.kind(), "bike");
    assert_eq!(gap.table().table().to_string(), "bike");
    assert_eq!(
        gap.to_string(),
        "a row in `bike` for every base row of the kind `bike`"
    );
    assert!(validate.gap().is_none());

    run(&ctx.db, added).await?;
    let db = ctx.db.get().await?;
    let error = db
        .execute(
            "INSERT INTO fleet (kind, name) VALUES ('bike', 'ghost')",
            &[],
        )
        .await
        .expect_err("a new bike without its row is refused");
    assert_eq!(
        refused(&error),
        (
            Some(SqlState::FOREIGN_KEY_VIOLATION),
            Some("fk-fleet-bike_id".to_owned())
        )
    );

    db.execute("INSERT INTO bike (id, gears) VALUES ($1, 21)", &[&first])
        .await?;
    let error = run(&ctx.db, validate)
        .await
        .expect_err("a bike still without its row refuses the change");
    assert_eq!(
        refused(&error),
        (
            Some(SqlState::FOREIGN_KEY_VIOLATION),
            Some("fk-fleet-bike_id".to_owned())
        )
    );

    db.execute("INSERT INTO bike (id, gears) VALUES ($1, 3)", &[&second])
        .await?;
    run(&ctx.db, validate).await?;
    let db = ctx.db.get().await?;
    let validated: bool = db
        .query_one(
            "SELECT convalidated FROM pg_constraint WHERE conname = 'fk-fleet-bike_id'",
            &[],
        )
        .await?
        .get(0);
    assert!(validated);

    let bikes: Vec<FleetBike> = FleetBike::find()
        .kind(FleetBikeKind::Bike)
        .order_by_asc(fleet_bike::Column::Id)
        .all(&db)
        .await?
        .into_iter()
        .map(|(_, value)| value)
        .collect();
    assert_eq!(
        bikes,
        [
            FleetBike::Bike(bike::Model {
                id: first,
                gears: 21
            }),
            FleetBike::Bike(bike::Model {
                id: second,
                gears: 3
            }),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.evolve+1/test]    a kind's table is dropped with the base's generated key columns and foreign key for it, in one step, after which its rows read and write as the unit kind
#[pgorm_macros::test]
async fn a_kind_loses_its_table_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_evolve_remove_table").await;
    create::<FleetBike>(&ctx.db).await?;
    let db = ctx.db.get().await?;
    let (bike, _) = FleetBike::insert(
        fleet_bike::ActiveModel {
            name: set("tourer"),
            ..Default::default()
        },
        FleetBikeActiveModel::Bike(bike::ActiveModel {
            gears: set(21_i16),
            ..Default::default()
        }),
    )?
    .exec_returning_model(&db)
    .await?;

    let change = Schema::new()
        .remove_disjoint_kind_table::<FleetOne>(FleetOneKind::Bike, Name::runtime("bike"))?;
    assert_eq!(change.steps().len(), 1);
    run_change(&ctx.db, &change).await?;
    let db = ctx.db.get().await?;
    let gone: bool = db
        .query_one("SELECT to_regclass('bike') IS NULL", &[])
        .await?
        .get(0);
    assert!(gone);
    assert_eq!(
        columns(&db, "fleet").await?,
        ["id", "kind", "name", "car_id"]
    );

    FleetOne::insert(
        fleet_one::ActiveModel {
            name: set("racer"),
            ..Default::default()
        },
        FleetOneActiveModel::Bike,
    )?
    .exec_returning_model(&db)
    .await?;
    let read: Vec<(i32, FleetOne)> = FleetOne::find()
        .order_by_asc(fleet_one::Column::Id)
        .all(&db)
        .await?
        .into_iter()
        .map(|(base, value)| (base.id, value))
        .collect();
    assert_eq!(read.len(), 2);
    assert_eq!(read[0], (bike.id, FleetOne::Bike));
    assert_eq!(read[1].1, FleetOne::Bike);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.evolve+1/test]    a schema-qualified kind type with a mixed-case, spaced name takes every change: a value added, a kind's table dropped, the value removed by recreating, and the kind added again by recreating, the entity reading and writing after each
#[pgorm_macros::test]
async fn a_quoted_schema_qualified_kind_type_changes_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_evolve_quoted").await;
    ctx.db
        .get()
        .await?
        .execute(r#"CREATE SCHEMA "Fleet Yard""#, &[])
        .await?;
    create::<YardOne>(&ctx.db).await?;
    let db = ctx.db.get().await?;
    YardOne::insert(
        yard_one::ActiveModel {
            name: set("sedan"),
            ..Default::default()
        },
        YardOneActiveModel::Car(car::ActiveModel {
            seats: set(5_i16),
            ..Default::default()
        }),
    )?
    .exec_returning_model(&db)
    .await?;
    const TYPE: &str = r#""Fleet Yard"."Yard Kind""#;

    run_change(
        &ctx.db,
        &Schema::new().add_disjoint_kind::<YardVan>(YardVanKind::Van)?,
    )
    .await?;
    let db = ctx.db.get().await?;
    assert_eq!(values(&db, TYPE).await?, ["car", "van", "bike"]);
    YardVan::insert(
        yard_van::ActiveModel {
            name: set("camper"),
            ..Default::default()
        },
        YardVanActiveModel::Van(van::ActiveModel {
            doors: set(4_i16),
            ..Default::default()
        }),
    )?
    .exec_returning_model(&db)
    .await?;
    assert_eq!(YardVan::find().all(&db).await?.len(), 2);

    db.execute("DELETE FROM yard WHERE kind = 'van'", &[])
        .await?;
    run_change(
        &ctx.db,
        &Schema::new().remove_disjoint_kind_table::<YardVanUnit>(
            YardVanUnitKind::Van,
            Name::runtime("van"),
        )?,
    )
    .await?;
    run_change(
        &ctx.db,
        &Schema::new().remove_disjoint_kind::<YardOne>("van")?,
    )
    .await?;
    let db = ctx.db.get().await?;
    assert_eq!(values(&db, TYPE).await?, ["car", "bike"]);
    YardOne::insert(
        yard_one::ActiveModel {
            name: set("tourer"),
            ..Default::default()
        },
        YardOneActiveModel::Bike,
    )?
    .exec_returning_model(&db)
    .await?;
    assert_eq!(YardOne::find().all(&db).await?.len(), 2);

    run_change(
        &ctx.db,
        &Schema::new().add_disjoint_kind_recreating::<YardVan>(YardVanKind::Van)?,
    )
    .await?;
    let db = ctx.db.get().await?;
    assert_eq!(values(&db, TYPE).await?, ["car", "van", "bike"]);
    YardVan::insert(
        yard_van::ActiveModel {
            name: set("camper"),
            ..Default::default()
        },
        YardVanActiveModel::Van(van::ActiveModel {
            doors: set(2_i16),
            ..Default::default()
        }),
    )?
    .exec_returning_model(&db)
    .await?;
    let kinds: Vec<YardVanKind> = YardVan::find()
        .order_by_asc(yard_van::Column::Id)
        .all(&db)
        .await?
        .iter()
        .map(|(_, value)| value.kind())
        .collect();
    assert_eq!(
        kinds,
        [YardVanKind::Car, YardVanKind::Bike, YardVanKind::Van]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

const VAN_TABLE: &str = r#"CREATE TABLE "van" ( "id" integer NOT NULL, "doors" smallint NOT NULL, "kind" fleet_kind GENERATED ALWAYS AS ('van') STORED, PRIMARY KEY ("id"), CONSTRAINT "fk-van-id-kind" FOREIGN KEY ("id", "kind") REFERENCES "fleet" ("id", "kind") ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE )"#;

// [spec:pgorm:sem:disjoint.evolve+1/test]    a kind with a table is added in two steps, the value after the kind declared before it, then its table as `create_disjoint` writes it and the base's generated key columns and foreign key for it, under the names `create_disjoint` gives them; a kind declared first is added before the one after it
#[test]
fn kind_added_as_value_then_table() {
    let change = Schema::new()
        .add_disjoint_kind::<FleetVan>(FleetVanKind::Van)
        .expect("the change is written");
    assert_eq!(
        rendered(&change),
        [
            vec![r#"ALTER TYPE "fleet_kind" ADD VALUE 'van' AFTER 'car'"#.to_owned()],
            vec![
                VAN_TABLE.to_owned(),
                [
                    r#"ALTER TABLE "fleet" ADD COLUMN "van_id" integer"#,
                    r#"GENERATED ALWAYS AS ((CASE WHEN ("kind" = 'van') THEN "id" END)) STORED,"#,
                    r#"ADD CONSTRAINT "fk-fleet-van_id" FOREIGN KEY ("van_id")"#,
                    r#"REFERENCES "van" ("id") DEFERRABLE INITIALLY IMMEDIATE"#,
                ]
                .join(" "),
            ],
        ]
    );
    let created: Vec<String> = Schema::new()
        .create_disjoint::<FleetVan>()
        .expect("the schema is written")
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(created.contains(&VAN_TABLE.to_owned()), "{created:#?}");

    let first = Schema::new()
        .add_disjoint_kind::<FleetTram>(FleetTramKind::Tram)
        .expect("the change is written");
    assert_eq!(
        rendered(&first),
        [[r#"ALTER TYPE "fleet_kind" ADD VALUE 'tram' BEFORE 'car'"#]]
    );
}

// [spec:pgorm:sem:disjoint.evolve+1/test]    the recreating route is one step: a new enum `{type}_next` of every value in order, each kind table's kind column dropped with its foreign key, the base's generated key columns, their keys and its UNIQUE key dropped and its kind column retyped USING its text after its default, the old type dropped and the new one renamed, then every constraint and generated column written again as `create_disjoint` writes it, the new kind's table among them
#[test]
fn a_kind_added_by_recreating_rewrites_every_constraint() {
    let change = Schema::new()
        .add_disjoint_kind_recreating::<FleetVan>(FleetVanKind::Van)
        .expect("the change is written");
    assert_eq!(
        rendered(&change),
        [vec![
            r#"CREATE TYPE "fleet_kind_next" AS ENUM ('car', 'van', 'bike')"#.to_owned(),
            r#"ALTER TABLE "car" DROP CONSTRAINT "fk-car-id-kind", DROP COLUMN "kind""#.to_owned(),
            [
                r#"ALTER TABLE "fleet" DROP CONSTRAINT "uq-fleet-id-kind","#,
                r#"DROP CONSTRAINT "fk-fleet-car_id", DROP COLUMN "car_id","#,
                r#"ALTER COLUMN "kind" DROP DEFAULT,"#,
                r#"ALTER COLUMN "kind" TYPE fleet_kind_next"#,
                r#"USING CAST(CAST("kind" AS text) AS fleet_kind_next)"#,
            ]
            .join(" "),
            r#"DROP TYPE "fleet_kind""#.to_owned(),
            r#"ALTER TYPE "fleet_kind_next" RENAME TO "fleet_kind""#.to_owned(),
            r#"ALTER TABLE "fleet" ADD CONSTRAINT "uq-fleet-id-kind" UNIQUE ("id", "kind")"#
                .to_owned(),
            [
                r#"ALTER TABLE "car" ADD COLUMN "kind" fleet_kind"#,
                r#"GENERATED ALWAYS AS ('car') STORED,"#,
                r#"ADD CONSTRAINT "fk-car-id-kind" FOREIGN KEY ("id", "kind")"#,
                r#"REFERENCES "fleet" ("id", "kind")"#,
                r#"ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE"#,
            ]
            .join(" "),
            VAN_TABLE.to_owned(),
            [
                r#"ALTER TABLE "fleet" ADD COLUMN "car_id" integer"#,
                r#"GENERATED ALWAYS AS ((CASE WHEN ("kind" = 'car') THEN "id" END)) STORED,"#,
                r#"ADD COLUMN "van_id" integer"#,
                r#"GENERATED ALWAYS AS ((CASE WHEN ("kind" = 'van') THEN "id" END)) STORED,"#,
                r#"ADD CONSTRAINT "fk-fleet-car_id" FOREIGN KEY ("car_id")"#,
                r#"REFERENCES "car" ("id") DEFERRABLE INITIALLY IMMEDIATE,"#,
                r#"ADD CONSTRAINT "fk-fleet-van_id" FOREIGN KEY ("van_id")"#,
                r#"REFERENCES "van" ("id") DEFERRABLE INITIALLY IMMEDIATE"#,
            ]
            .join(" "),
        ]]
    );
}

// [spec:pgorm:sem:disjoint.evolve+1/test]    removing a value is one step whose first statement adds the CHECK refusing it, named as the schema's names are, and whose next drops it, before the recreating route without the value
#[test]
fn kind_removed_after_check_refusing_it() {
    let change = Schema::new()
        .remove_disjoint_kind::<FleetOne>("scooter")
        .expect("the change is written");
    let [step] = rendered(&change).try_into().expect("one step");
    assert_eq!(
        step[..3],
        [
            r#"ALTER TABLE "fleet" ADD CONSTRAINT "ck-fleet-kind-scooter" CHECK ("kind" <> 'scooter')"#,
            r#"ALTER TABLE "fleet" DROP CONSTRAINT "ck-fleet-kind-scooter""#,
            r#"CREATE TYPE "fleet_kind_next" AS ENUM ('car', 'bike')"#,
        ]
    );
    assert_eq!(step.len(), 10);
}

// [spec:pgorm:sem:disjoint.evolve+1/test]    a unit kind gains its table in two steps, the first ending in a gap naming the rows the program writes, its foreign key added NOT VALID, the second validating it; a kind loses its table in one step dropping the base's foreign key and generated key columns for it and then the table
#[test]
fn a_kind_table_is_added_and_dropped() {
    let change = Schema::new()
        .add_disjoint_kind_table::<FleetBike>(FleetBikeKind::Bike)
        .expect("the change is written");
    assert_eq!(
        rendered(&change),
        [
            vec![
                [
                    r#"CREATE TABLE "bike" ( "id" integer NOT NULL, "gears" smallint NOT NULL,"#,
                    r#""kind" fleet_kind GENERATED ALWAYS AS ('bike') STORED, PRIMARY KEY ("id"),"#,
                    r#"CONSTRAINT "fk-bike-id-kind" FOREIGN KEY ("id", "kind")"#,
                    r#"REFERENCES "fleet" ("id", "kind")"#,
                    r#"ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE )"#,
                ]
                .join(" "),
                [
                    r#"ALTER TABLE "fleet" ADD COLUMN "bike_id" integer"#,
                    r#"GENERATED ALWAYS AS ((CASE WHEN ("kind" = 'bike') THEN "id" END)) STORED,"#,
                    r#"ADD CONSTRAINT "fk-fleet-bike_id" FOREIGN KEY ("bike_id")"#,
                    r#"REFERENCES "bike" ("id") DEFERRABLE INITIALLY IMMEDIATE NOT VALID"#,
                ]
                .join(" "),
            ],
            vec![r#"ALTER TABLE "fleet" VALIDATE CONSTRAINT "fk-fleet-bike_id""#.to_owned()],
        ]
    );
    let gaps: Vec<Option<String>> = change
        .steps()
        .iter()
        .map(|step| step.gap().map(ToString::to_string))
        .collect();
    assert_eq!(
        gaps,
        [
            Some("a row in `bike` for every base row of the kind `bike`".to_owned()),
            None
        ]
    );

    let change = Schema::new()
        .remove_disjoint_kind_table::<FleetOne>(FleetOneKind::Bike, Name::runtime("bike"))
        .expect("the change is written");
    assert_eq!(
        rendered(&change),
        [[
            r#"ALTER TABLE "fleet" DROP CONSTRAINT "fk-fleet-bike_id", DROP COLUMN "bike_id""#,
            r#"DROP TABLE "bike""#,
        ]]
    );
}

// [spec:pgorm:sem:disjoint.evolve+1/test]    each function refuses a change the declaration does not describe as `Error::Type` naming the kind — a kind added that it does not hold, a value removed that it holds, a table given to a kind it declares without one and dropped from one it declares with one — and runs `check_disjoint` first, returning its refusal as its own
#[test]
fn changes_the_declaration_does_not_describe_are_refused() {
    let schema = Schema::new();
    let one = DisjointShape::of::<FleetOne>();
    let not_held = "`van` is no kind of the disjoint entity over `fleet`, whose kinds are \
                    `car`, `bike`";
    assert_eq!(refusal(one.add_kind("van")), not_held);
    assert_eq!(refusal(one.add_kind_recreating("van")), not_held);
    assert_eq!(refusal(one.add_kind_table("van")), not_held);
    assert_eq!(
        refusal(one.remove_kind_table("van", TableName::Table(Name::runtime("van")))),
        not_held
    );
    assert_eq!(
        refusal(schema.remove_disjoint_kind::<FleetVan>("van")),
        "the disjoint entity over `fleet` declares the kind `van`, so it is not removed"
    );
    assert_eq!(
        refusal(schema.add_disjoint_kind_table::<FleetOne>(FleetOneKind::Bike)),
        "the disjoint entity over `fleet` declares the kind `bike` without a table, so it is \
         given none"
    );
    assert_eq!(
        refusal(
            schema.remove_disjoint_kind_table::<FleetBike>(
                FleetBikeKind::Bike,
                Name::runtime("bike")
            )
        ),
        "the disjoint entity over `fleet` declares the kind `bike` with the table `bike`, so \
         its table is not dropped"
    );

    let checked = refusal(check_disjoint::<LooseFleet>());
    assert_eq!(
        refusal(schema.add_disjoint_kind::<LooseFleet>(LooseFleetKind::Bike)),
        checked
    );
    assert_eq!(
        refusal(schema.add_disjoint_kind_recreating::<LooseFleet>(LooseFleetKind::Bike)),
        checked
    );
    assert_eq!(
        refusal(schema.remove_disjoint_kind::<LooseFleet>("van")),
        checked
    );
    assert_eq!(
        refusal(schema.add_disjoint_kind_table::<LooseFleet>(LooseFleetKind::Bike)),
        checked
    );
    assert_eq!(
        refusal(
            schema.remove_disjoint_kind_table::<LooseFleet>(
                LooseFleetKind::Car,
                Name::runtime("car")
            )
        ),
        checked
    );
}

// [spec:pgorm:sem:disjoint.evolve+1/test]    the base's kind column loses its default before the retype, which PostgreSQL would refuse to cast, and gets the declaration's back after the rename, of the new type
#[pgorm_macros::test]
async fn a_kind_columns_default_survives_recreating_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_evolve_default").await;
    create::<KeptOne>(&ctx.db).await?;

    let change = Schema::new().add_disjoint_kind_recreating::<KeptVan>(KeptVanKind::Van)?;
    let [step] = rendered(&change).try_into().expect("one step");
    assert_eq!(
        step[5],
        [
            r#"ALTER TABLE "fleet" ADD CONSTRAINT "uq-fleet-id-kind" UNIQUE ("id", "kind"),"#,
            r#"ALTER COLUMN "kind" SET DEFAULT 'bike'"#,
        ]
        .join(" ")
    );
    run_change(&ctx.db, &change).await?;

    let db = ctx.db.get().await?;
    assert_eq!(values(&db, "fleet_kind").await?, ["car", "van", "bike"]);
    let kind: String = db
        .query_one(
            "INSERT INTO fleet (name) VALUES ('tourer') RETURNING kind::text",
            &[],
        )
        .await?
        .get(0);
    assert_eq!(kind, "bike");
    let default: String = db
        .query_one(
            "SELECT pg_get_expr(adbin, adrelid) FROM pg_attrdef \
             WHERE adrelid = 'fleet'::regclass AND adnum = 2",
            &[],
        )
        .await?
        .get(0);
    assert_eq!(default, "'bike'::fleet_kind");

    drop(db);
    ctx.delete().await;
    Ok(())
}
