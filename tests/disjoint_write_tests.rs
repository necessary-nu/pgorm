#![allow(unused_imports, dead_code)]

//! A disjoint entity's writes on a live server: inserts of each kind, the
//! updates of either row or both, changes of kind between the kinds with a
//! table and the one without, each returning what a read of the row gives;
//! the refusals that send nothing and the misses that change nothing; the
//! writes inside a transaction, a concurrent change of kind, a composite key
//! in a schema of its own, and kind columns written under their casts.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo nextest run --test disjoint_write_tests`

pub mod common;
pub use common::{TestContext, setup::*};

use pgorm::tests_cfg::{
    car, flat,
    listing::{self, Listing, ListingActiveModel, ListingKind},
    truck,
    vehicle::{self, Vehicle, VehicleActiveModel, VehicleKind},
};
use pgorm::{
    ConnectionTrait, DatabaseConnection, DatabasePool, Disjoint, Error, Schema, TransactionTrait,
    entity::prelude::*,
};
use pretty_assertions::assert_eq;
use std::time::Duration;

/// A grade of [`priced`] stock, an enum column of a kind's table.
pub mod grade {
    use pgorm::entity::prelude::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, ActiveEnum)]
    #[pgorm(enum_name = "grade")]
    pub enum Grade {
        #[pgorm(string_value = "fine")]
        Fine,
        #[pgorm(string_value = "coarse")]
        Coarse,
    }
}

/// The kind `priced` of [`Stock`]: a `money` and an enum column, each
/// written through its column's cast.
pub mod priced {
    use super::grade::Grade;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "priced")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub stock_id: i64,
        #[pgorm(column_type = ColumnType::Money)]
        pub price: Decimal,
        pub grade: Grade,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// The base of [`Stock`], its key a database-generated `bigint`.
pub mod stock {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "stock")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i64,
        pub kind: super::StockKind,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// Stock priced in a table of its own, or given away.
#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = stock::Entity)]
pub enum Stock {
    Priced(priced::Model),
    Free,
}

/// Run the statements `create_disjoint` returns, in order.
async fn create<D: Disjoint>(db: &DatabaseConnection) -> Result<(), Error> {
    for statement in Schema::new().create_disjoint::<D>()? {
        db.execute(&statement.to_string(), &[]).await?;
    }
    Ok(())
}

/// Every row of the vehicle tables, each in key order: what a write that
/// changes nothing leaves exactly as it was.
#[derive(Debug, PartialEq)]
struct Tables {
    vehicles: Vec<vehicle::Model>,
    cars: Vec<car::Model>,
    trucks: Vec<truck::Model>,
}

async fn tables<C: ConnectionTrait>(db: &C) -> Result<Tables, Error> {
    Ok(Tables {
        vehicles: vehicle::Entity::find()
            .order_by_asc(vehicle::Column::Id)
            .all(db)
            .await?,
        cars: car::Entity::find()
            .order_by_asc(car::Column::Id)
            .all(db)
            .await?,
        trucks: truck::Entity::find()
            .order_by_asc(truck::Column::VehicleId)
            .all(db)
            .await?,
    })
}

fn named(name: &str) -> vehicle::ActiveModel {
    vehicle::ActiveModel {
        name: set(name),
        ..Default::default()
    }
}

fn keyed(id: i32) -> vehicle::ActiveModel {
    vehicle::ActiveModel {
        id: ActiveValue::Unchanged(id),
        ..Default::default()
    }
}

fn renamed(id: i32, name: &str) -> vehicle::ActiveModel {
    vehicle::ActiveModel {
        name: set(name),
        ..keyed(id)
    }
}

fn seats(seats: i16) -> VehicleActiveModel {
    VehicleActiveModel::Car(car::ActiveModel {
        seats: set(seats),
        ..Default::default()
    })
}

fn truck(axles: i16, payload_kg: i32) -> VehicleActiveModel {
    VehicleActiveModel::Truck(truck::ActiveModel {
        axles: set(axles),
        payload_kg: set(payload_kg),
        ..Default::default()
    })
}

fn untouched_car() -> VehicleActiveModel {
    VehicleActiveModel::Car(car::ActiveModel {
        ..Default::default()
    })
}

fn base(id: i32, kind: VehicleKind, name: &str) -> vehicle::Model {
    vehicle::Model {
        id,
        kind,
        name: name.to_owned(),
    }
}

/// Write one vehicle of each kind through `Vehicle::insert`.
async fn fleet<C: ConnectionTrait>(db: &C) -> Result<[(vehicle::Model, Vehicle); 3], Error> {
    let mini = Vehicle::insert(named("mini"), seats(4))?
        .exec_returning_model(db)
        .await?;
    let lorry = Vehicle::insert(named("lorry"), truck(3, 18_000))?
        .exec_returning_model(db)
        .await?;
    let bmx = Vehicle::insert(named("bmx"), VehicleActiveModel::Bike)?
        .exec_returning_model(db)
        .await?;
    Ok([mini, lorry, bmx])
}

/// Assert that `written` is what a read of its row gives.
async fn assert_read<C: ConnectionTrait>(
    db: &C,
    written: &(vehicle::Model, Vehicle),
) -> Result<(), Error> {
    assert_eq!(&Vehicle::find_by_id(written.0.id).one(db).await?, written);
    Ok(())
}

// [spec:pgorm:sem:disjoint.write+1/test]    `insert` writes a car and a truck with their rows, the key the database generates for the base reaching each kind's row, and a bike as its base row alone, each returning what a read of the row gives
#[pgorm_macros::test]
async fn insert_writes_every_kind_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_write_insert").await;
    let db = ctx.db.get().await?;
    create::<Vehicle>(&db).await?;

    let [mini, lorry, bmx] = fleet(&db).await?;
    let ids = [mini.0.id, lorry.0.id, bmx.0.id];
    assert!(ids[0] < ids[1] && ids[1] < ids[2], "{ids:?}");
    assert_eq!(
        mini,
        (
            base(ids[0], VehicleKind::Car, "mini"),
            Vehicle::Car(car::Model {
                id: ids[0],
                seats: 4
            })
        )
    );
    assert_eq!(
        lorry,
        (
            base(ids[1], VehicleKind::Truck, "lorry"),
            Vehicle::Truck(truck::Model {
                vehicle_id: ids[1],
                axles: 3,
                payload_kg: 18_000,
            })
        )
    );
    assert_eq!(bmx, (base(ids[2], VehicleKind::Bike, "bmx"), Vehicle::Bike));
    for written in [&mini, &lorry, &bmx] {
        assert_read(&db, written).await?;
    }
    let all = tables(&db).await?;
    assert_eq!(all.cars.len(), 1);
    assert_eq!(all.trucks.len(), 1);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.write+1/test]    `insert` refuses, as an `Error::Type` naming the table and column and with nothing written, a base kind column `Set` or `Unchanged` and a kind row's key `Set` or `Unchanged`
#[pgorm_macros::test]
async fn insert_refuses_kind_and_key_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_write_insert_refused").await;
    let db = ctx.db.get().await?;
    create::<Vehicle>(&db).await?;

    let kinded = [
        vehicle::ActiveModel {
            kind: set(VehicleKind::Car),
            ..named("set")
        },
        vehicle::ActiveModel {
            kind: ActiveValue::Unchanged(VehicleKind::Car),
            ..named("unchanged")
        },
    ];
    for base in kinded {
        match Vehicle::insert(base, seats(4)) {
            Err(Error::Type(message)) => {
                assert!(message.contains("`vehicle`.`kind`"), "{message}")
            }
            other => panic!("expected the kind column refused, got {other:?}"),
        }
    }
    let keys = [
        car::ActiveModel {
            id: set(1),
            seats: set(4_i16),
        },
        car::ActiveModel {
            id: ActiveValue::Unchanged(1),
            seats: set(4_i16),
        },
    ];
    for key in keys {
        match Vehicle::insert(named("keyed"), VehicleActiveModel::Car(key)) {
            Err(Error::Type(message)) => {
                assert!(message.contains("`car`.`id`"), "{message}");
                assert!(message.contains("kind `car`"), "{message}");
            }
            other => panic!("expected the kind row's key refused, got {other:?}"),
        }
    }
    assert_eq!(
        tables(&db).await?,
        Tables {
            vehicles: vec![],
            cars: vec![],
            trucks: vec![],
        }
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.write+1/test]    `update` writes the set columns of the base alone, of the kind's row alone and of both, found by the base's key, and with nothing set reads the row back; each returns what a read of the row gives, and the other rows are untouched
#[pgorm_macros::test]
async fn update_writes_either_row_or_both_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_write_update").await;
    let db = ctx.db.get().await?;
    create::<Vehicle>(&db).await?;
    let [mini, lorry, bmx] = fleet(&db).await?;
    let id = mini.0.id;

    let renamed_only = Vehicle::update(renamed(id, "cooper"), untouched_car())?
        .exec_returning_model(&db)
        .await?;
    assert_eq!(
        renamed_only,
        (
            base(id, VehicleKind::Car, "cooper"),
            Vehicle::Car(car::Model { id, seats: 4 })
        )
    );
    assert_read(&db, &renamed_only).await?;

    let seated_only = Vehicle::update(keyed(id), seats(5))?
        .exec_returning_model(&db)
        .await?;
    assert_eq!(
        seated_only,
        (
            base(id, VehicleKind::Car, "cooper"),
            Vehicle::Car(car::Model { id, seats: 5 })
        )
    );
    assert_read(&db, &seated_only).await?;

    let both = Vehicle::update(renamed(id, "clubman"), seats(6))?
        .exec_returning_model(&db)
        .await?;
    assert_eq!(
        both,
        (
            base(id, VehicleKind::Car, "clubman"),
            Vehicle::Car(car::Model { id, seats: 6 })
        )
    );
    assert_read(&db, &both).await?;

    let nothing = Vehicle::update(keyed(id), untouched_car())?
        .exec_returning_model(&db)
        .await?;
    assert_eq!(nothing, both);

    let bike = Vehicle::update(renamed(bmx.0.id, "fixie"), VehicleActiveModel::Bike)?
        .exec_returning_model(&db)
        .await?;
    assert_eq!(
        bike,
        (base(bmx.0.id, VehicleKind::Bike, "fixie"), Vehicle::Bike)
    );
    assert_read(&db, &lorry).await?;

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.write+1/test]    `update` of a row as another kind, and of a key no row has, is `RecordNotFound` with every row as it was; the kind column `Set`, a kind row's key `Set` and a `NotSet` base key are refused before anything is sent
#[pgorm_macros::test]
async fn update_misses_change_nothing_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_write_update_missed").await;
    let db = ctx.db.get().await?;
    create::<Vehicle>(&db).await?;
    let [mini, lorry, bmx] = fleet(&db).await?;
    let before = tables(&db).await?;

    let misses = [
        Vehicle::update(renamed(mini.0.id, "lorry?"), truck(9, 9))?,
        Vehicle::update(keyed(mini.0.id), truck(9, 9))?,
        Vehicle::update(renamed(mini.0.id, "bike?"), VehicleActiveModel::Bike)?,
        Vehicle::update(renamed(bmx.0.id, "car?"), seats(9))?,
        Vehicle::update(renamed(lorry.0.id, "car?"), untouched_car())?,
        Vehicle::update(renamed(-1, "nobody"), seats(9))?,
        Vehicle::update(keyed(-1), untouched_car())?,
    ];
    for miss in misses {
        assert_eq!(
            miss.exec_returning_model(&db).await,
            Err(Error::RecordNotFound)
        );
    }
    assert_eq!(tables(&db).await?, before);

    let rekinded = vehicle::ActiveModel {
        kind: set(VehicleKind::Truck),
        ..renamed(mini.0.id, "lorry?")
    };
    assert!(
        matches!(Vehicle::update(rekinded, truck(9, 9)), Err(Error::Type(message)) if message.contains("`vehicle`.`kind`"))
    );
    let rekeyed = VehicleActiveModel::Car(car::ActiveModel {
        id: set(bmx.0.id),
        seats: set(9_i16),
    });
    assert!(
        matches!(Vehicle::update(keyed(mini.0.id), rekeyed), Err(Error::Type(message)) if message.contains("`car`.`id`"))
    );
    assert_eq!(
        Vehicle::update(named("keyless"), seats(9)).err(),
        Some(Error::PrimaryKeyNotSet)
    );
    assert_eq!(tables(&db).await?, before);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.write+1/test]    `change_kind` moves a row from one kind's table to another's, from a kind with a table to the unit kind and back, deleting the old row and inserting the new one keyed by the base row, each returning what a read of the row gives
#[pgorm_macros::test]
async fn change_kind_moves_rows_between_tables_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_write_change_kind").await;
    let db = ctx.db.get().await?;
    create::<Vehicle>(&db).await?;
    let [mini, lorry, _] = fleet(&db).await?;
    let id = mini.0.id;

    let trucked = Vehicle::change_kind(id, truck(2, 1_000))?
        .exec_returning_model(&db)
        .await?;
    assert_eq!(
        trucked,
        (
            base(id, VehicleKind::Truck, "mini"),
            Vehicle::Truck(truck::Model {
                vehicle_id: id,
                axles: 2,
                payload_kg: 1_000,
            })
        )
    );
    assert_read(&db, &trucked).await?;
    assert_eq!(
        car::Entity::find_by_id(id).one(&db).await,
        Err(Error::RecordNotFound)
    );

    let biked = Vehicle::change_kind(id, VehicleActiveModel::Bike)?
        .exec_returning_model(&db)
        .await?;
    assert_eq!(biked, (base(id, VehicleKind::Bike, "mini"), Vehicle::Bike));
    assert_read(&db, &biked).await?;

    let carred = Vehicle::change_kind(id, seats(7))?
        .exec_returning_model(&db)
        .await?;
    assert_eq!(
        carred,
        (
            base(id, VehicleKind::Car, "mini"),
            Vehicle::Car(car::Model { id, seats: 7 })
        )
    );
    assert_read(&db, &carred).await?;

    let all = tables(&db).await?;
    assert_eq!(all.cars, vec![car::Model { id, seats: 7 }]);
    assert_eq!(
        all.trucks,
        vec![truck::Model {
            vehicle_id: lorry.0.id,
            axles: 3,
            payload_kg: 18_000,
        }]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.write+1/test]    `change_kind` to the row's own kind, with a table or without, and of a key no row has, is `RecordNotFound` with every row as it was; a key in the new kind's row is refused before anything is sent
#[pgorm_macros::test]
async fn change_kind_misses_change_nothing_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_write_change_missed").await;
    let db = ctx.db.get().await?;
    create::<Vehicle>(&db).await?;
    let [mini, lorry, bmx] = fleet(&db).await?;
    let before = tables(&db).await?;

    let misses = [
        Vehicle::change_kind(mini.0.id, seats(9))?,
        Vehicle::change_kind(lorry.0.id, truck(9, 9))?,
        Vehicle::change_kind(bmx.0.id, VehicleActiveModel::Bike)?,
        Vehicle::change_kind(-1, seats(9))?,
        Vehicle::change_kind(-1, VehicleActiveModel::Bike)?,
    ];
    for miss in misses {
        assert_eq!(
            miss.exec_returning_model(&db).await,
            Err(Error::RecordNotFound)
        );
    }
    assert_eq!(tables(&db).await?, before);

    let keyed_truck = VehicleActiveModel::Truck(truck::ActiveModel {
        vehicle_id: set(mini.0.id),
        axles: set(2_i16),
        payload_kg: set(1),
    });
    assert!(
        matches!(Vehicle::change_kind(mini.0.id, keyed_truck), Err(Error::Type(message)) if message.contains("`truck`.`vehicle_id`"))
    );
    assert_eq!(tables(&db).await?, before);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.write+1/test]    inside a transaction each write is one statement too: inserts, updates and changes of kind commit together, a miss inside one leaves the transaction usable, and a rollback undoes them all
#[pgorm_macros::test]
async fn writes_compose_inside_transaction_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_write_transaction").await;
    let mut db = ctx.db.get().await?;
    create::<Vehicle>(&db).await?;

    let committed = db
        .transaction(async |tx| {
            let [mini, lorry, bmx] = fleet(tx).await?;
            Vehicle::update(renamed(mini.0.id, "cooper"), seats(5))?
                .exec_returning_model(tx)
                .await?;
            assert_eq!(
                Vehicle::update(keyed(lorry.0.id), seats(9))?
                    .exec_returning_model(tx)
                    .await,
                Err(Error::RecordNotFound)
            );
            Vehicle::change_kind(bmx.0.id, truck(2, 400))?
                .exec_returning_model(tx)
                .await?;
            Vehicle::change_kind(lorry.0.id, VehicleActiveModel::Bike)?
                .exec_returning_model(tx)
                .await?;
            Ok::<_, Error>([mini.0.id, lorry.0.id, bmx.0.id])
        })
        .await
        .expect("the transaction commits");
    let [mini, lorry, bmx] = committed;
    assert_eq!(
        Vehicle::find()
            .order_by_asc(vehicle::Column::Id)
            .all(&db)
            .await?,
        vec![
            (
                base(mini, VehicleKind::Car, "cooper"),
                Vehicle::Car(car::Model { id: mini, seats: 5 })
            ),
            (base(lorry, VehicleKind::Bike, "lorry"), Vehicle::Bike),
            (
                base(bmx, VehicleKind::Truck, "bmx"),
                Vehicle::Truck(truck::Model {
                    vehicle_id: bmx,
                    axles: 2,
                    payload_kg: 400,
                })
            ),
        ]
    );
    let before = tables(&db).await?;

    let outcome = db
        .transaction(async |tx| {
            Vehicle::insert(named("ghost"), seats(2))?
                .exec_returning_model(tx)
                .await?;
            Vehicle::update(renamed(mini, "ghost"), seats(1))?
                .exec_returning_model(tx)
                .await?;
            Vehicle::change_kind(bmx, VehicleActiveModel::Bike)?
                .exec_returning_model(tx)
                .await?;
            roll_back::<()>()
        })
        .await;
    assert_rolled_back(outcome);
    assert_eq!(tables(&db).await?, before);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// Wait, for up to 30 seconds, until a backend of this database other than
/// `db`'s own waits on a lock.
async fn await_lock_wait(db: &DatabaseConnection) -> Result<(), Error> {
    for _ in 0..3_000 {
        let waiting: i64 = db
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                 AND pid <> pg_backend_pid() AND wait_event_type = 'Lock'",
                &[],
            )
            .await?
            .get(0);
        if waiting > 0 {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no backend came to wait on a lock");
}

/// Run `update` on a connection of its own while another holds the row's
/// change of kind uncommitted, then commit the change.
async fn update_behind_kind_change(
    pool: &DatabasePool,
    id: i32,
    update: pgorm::WriteDisjoint<Vehicle>,
) -> Result<Result<(vehicle::Model, Vehicle), Error>, Error> {
    let mut changer = pool.get().await?;
    let watcher = pool.get().await?;
    let pool = pool.clone();
    let mut waiting = None;
    changer
        .transaction(async |tx| {
            Vehicle::change_kind(id, truck(2, 100))?
                .exec_returning_model(tx)
                .await?;
            waiting = Some(tokio::spawn(async move {
                let db = pool.get().await?;
                Ok::<_, Error>(update.exec_returning_model(&db).await)
            }));
            await_lock_wait(&watcher).await
        })
        .await
        .expect("the change of kind commits");
    waiting
        .expect("the update was started")
        .await
        .expect("the update's task completes")
}

// [spec:pgorm:sem:disjoint.write+1/test]    an update waiting on a change of the row's kind that then commits is `RecordNotFound`, whether it sets base columns or the kind's row alone, and the change stands
#[pgorm_macros::test]
async fn update_behind_committed_kind_change_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_write_concurrent").await;
    let db = ctx.db.get().await?;
    create::<Vehicle>(&db).await?;
    let [mini, lorry, _] = fleet(&db).await?;

    let kind_only = Vehicle::update(keyed(mini.0.id), seats(9))?;
    assert_eq!(
        update_behind_kind_change(&ctx.db, mini.0.id, kind_only).await?,
        Err(Error::RecordNotFound)
    );
    Vehicle::change_kind(mini.0.id, seats(4))?
        .exec_returning_model(&db)
        .await?;
    let both = Vehicle::update(renamed(mini.0.id, "late"), seats(9))?;
    assert_eq!(
        update_behind_kind_change(&ctx.db, mini.0.id, both).await?,
        Err(Error::RecordNotFound)
    );
    assert_eq!(
        Vehicle::find_by_id(mini.0.id).one(&db).await?,
        (
            base(mini.0.id, VehicleKind::Truck, "mini"),
            Vehicle::Truck(truck::Model {
                vehicle_id: mini.0.id,
                axles: 2,
                payload_kg: 100,
            })
        )
    );
    assert_read(&db, &lorry).await?;

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.write+1/test]    deleting through the base entity removes the base row and, by the cascade, its kind's row
#[pgorm_macros::test]
async fn base_delete_cascades_to_kind_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_write_delete").await;
    let db = ctx.db.get().await?;
    create::<Vehicle>(&db).await?;
    let [mini, lorry, bmx] = fleet(&db).await?;

    for gone in [mini.0.id, bmx.0.id] {
        assert_eq!(vehicle::Entity::delete_by_id(gone).exec(&db).await?, 1);
    }
    assert_eq!(
        tables(&db).await?,
        Tables {
            vehicles: vec![lorry.0.clone()],
            cars: vec![],
            trucks: vec![truck::Model {
                vehicle_id: lorry.0.id,
                axles: 3,
                payload_kg: 18_000,
            }],
        }
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.write+1/test]    a composite key in a schema of its own, its kind table's key named differently: insert, update and change of kind each pair the key column by column and return what a read of the row gives
#[pgorm_macros::test]
async fn composite_key_writes_pair_columns_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_write_composite").await;
    let db = ctx.db.get().await?;
    db.execute("CREATE SCHEMA market", &[]).await?;
    create::<Listing>(&db).await?;

    let read = async |key: (i32, i64)| Listing::find_by_id(key).one(&db).await;
    let loft = Listing::insert(
        listing::ActiveModel {
            tenant_id: set(7),
            id: set(1_i64),
            title: set("loft"),
            ..Default::default()
        },
        ListingActiveModel::Flat(flat::ActiveModel {
            rooms: set(3_i16),
            ..Default::default()
        }),
    )?
    .exec_returning_model(&db)
    .await?;
    let model = |category, title: &str| listing::Model {
        tenant_id: 7,
        id: 1,
        category,
        title: title.to_owned(),
    };
    let rooms = |rooms| {
        Listing::Flat(flat::Model {
            tenant: 7,
            listing: 1,
            rooms,
        })
    };
    assert_eq!(loft, (model(ListingKind::Flat, "loft"), rooms(3)));
    assert_eq!(read((7, 1)).await?, loft);
    let neighbour = Listing::insert(
        listing::ActiveModel {
            tenant_id: set(7),
            id: set(2_i64),
            title: set("plot"),
            ..Default::default()
        },
        ListingActiveModel::BuildingPlot,
    )?
    .exec_returning_model(&db)
    .await?;

    let keyed = listing::ActiveModel {
        tenant_id: ActiveValue::Unchanged(7),
        id: ActiveValue::Unchanged(1),
        title: set("attic"),
        ..Default::default()
    };
    let attic = Listing::update(
        keyed,
        ListingActiveModel::Flat(flat::ActiveModel {
            rooms: set(4_i16),
            ..Default::default()
        }),
    )?
    .exec_returning_model(&db)
    .await?;
    assert_eq!(attic, (model(ListingKind::Flat, "attic"), rooms(4)));
    assert_eq!(read((7, 1)).await?, attic);

    let plot = Listing::change_kind((7, 1_i64), ListingActiveModel::BuildingPlot)?
        .exec_returning_model(&db)
        .await?;
    assert_eq!(
        plot,
        (
            model(ListingKind::BuildingPlot, "attic"),
            Listing::BuildingPlot
        )
    );
    assert_eq!(read((7, 1)).await?, plot);
    let flat_again = Listing::change_kind(
        (7, 1_i64),
        ListingActiveModel::Flat(flat::ActiveModel {
            rooms: set(2_i16),
            ..Default::default()
        }),
    )?
    .exec_returning_model(&db)
    .await?;
    assert_eq!(flat_again, (model(ListingKind::Flat, "attic"), rooms(2)));
    assert_eq!(read((7, 1)).await?, flat_again);
    assert_eq!(read((7, 2)).await?, neighbour);
    assert_eq!(
        Listing::change_kind((8, 1_i64), ListingActiveModel::BuildingPlot)?
            .exec_returning_model(&db)
            .await,
        Err(Error::RecordNotFound)
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.write+1/test]    a kind table's `money` and enum columns are written under their casts by insert, update and change of kind, and read back as written, beside a base key the database generates as a `bigint`
#[pgorm_macros::test]
async fn kind_columns_write_under_casts_live() -> Result<(), Error> {
    use grade::Grade;

    let ctx = TestContext::new("disjoint_write_casts").await;
    let db = ctx.db.get().await?;
    let grade = Schema::new().create_enum_from_active_enum::<Grade>()?;
    db.execute(&grade.to_string(), &[]).await?;
    create::<Stock>(&db).await?;

    let label = |label: &str| stock::ActiveModel {
        label: set(label),
        ..Default::default()
    };
    let priced = |price: Decimal, grade: Grade| {
        StockActiveModel::Priced(priced::ActiveModel {
            price: set(price),
            grade: set(grade),
            ..Default::default()
        })
    };
    let flour = Stock::insert(label("flour"), priced(Decimal::new(1250, 2), Grade::Fine))?
        .exec_returning_model(&db)
        .await?;
    let id = flour.0.id;
    assert_eq!(
        flour.1,
        Stock::Priced(priced::Model {
            stock_id: id,
            price: Decimal::new(1250, 2),
            grade: Grade::Fine,
        })
    );
    assert_eq!(Stock::find_by_id(id).one(&db).await?, flour);

    let regraded = Stock::update(
        stock::ActiveModel {
            id: ActiveValue::Unchanged(id),
            ..Default::default()
        },
        priced(Decimal::new(999, 2), Grade::Coarse),
    )?
    .exec_returning_model(&db)
    .await?;
    assert_eq!(
        regraded.1,
        Stock::Priced(priced::Model {
            stock_id: id,
            price: Decimal::new(999, 2),
            grade: Grade::Coarse,
        })
    );
    assert_eq!(Stock::find_by_id(id).one(&db).await?, regraded);

    let free = Stock::insert(label("sample"), StockActiveModel::Free)?
        .exec_returning_model(&db)
        .await?;
    let sold = Stock::change_kind(free.0.id, priced(Decimal::new(5, 1), Grade::Fine))?
        .exec_returning_model(&db)
        .await?;
    assert_eq!(
        sold.1,
        Stock::Priced(priced::Model {
            stock_id: free.0.id,
            price: Decimal::new(5, 1),
            grade: Grade::Fine,
        })
    );
    assert_eq!(Stock::find_by_id(free.0.id).one(&db).await?, sold);

    drop(db);
    ctx.delete().await;
    Ok(())
}
