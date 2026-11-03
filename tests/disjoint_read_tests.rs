#![allow(unused_imports, dead_code)]

//! A disjoint entity's read on a live server: each kind's row decoded into
//! its variant, the read narrowed by key, kind, base and kind columns,
//! ordered, sliced, paged and read through a keyset cursor, more kinds than a
//! graph has slots, and the rows of a schema `Schema::create_disjoint` did
//! not write reported rather than read or dropped.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo nextest run --test disjoint_read_tests`

pub mod common;
pub use common::{TestContext, setup::*};

use futures::StreamExt;
use pgorm::tests_cfg::{
    car, flat,
    listing::{self, Listing},
    truck,
    vehicle::{self, Vehicle, VehicleKind},
};
use pgorm::{
    ConnectionTrait, DatabaseConnection, Disjoint, Error, RuntimeError, Schema, entity::prelude::*,
};
use pretty_assertions::assert_eq;
use std::num::NonZeroU64;

/// A kind entity of [`Shape`]: keyed by its shape's `id`, with a `name` and
/// `sides` of its own beside the base's `name`.
macro_rules! sided {
    ($module:ident, $table:literal) => {
        pub mod $module {
            use pgorm::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = $table)]
            pub struct Model {
                #[pgorm(primary_key, auto_increment = false)]
                pub id: i32,
                pub name: String,
                pub sides: i16,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }
    };
}

sided!(triangle, "triangle");
sided!(square, "square");
sided!(pentagon, "pentagon");
sided!(hexagon, "hexagon");
sided!(heptagon, "heptagon");
sided!(nonagon, "nonagon");

/// The sixth kind with a table, with a column long enough that its prefixed
/// result name, `s6_` and 60 bytes, is bounded to PostgreSQL's 63.
pub mod octagon {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "octagon")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub name: String,
        pub sides: i16,
        pub corners_counted_one_at_a_time_all_the_way_around_the_octagon: Option<i16>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// The base of [`Shape`], whose `name` every kind's table repeats.
pub mod shape {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "shape")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub kind: super::ShapeKind,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// Seven kinds with a table, one more than a graph's slots, and one without.
#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = shape::Entity)]
pub enum Shape {
    Triangle(triangle::Model),
    Square(square::Model),
    Pentagon(pentagon::Model),
    Hexagon(hexagon::Model),
    Heptagon(heptagon::Model),
    Octagon(octagon::Model),
    Nonagon(nonagon::Model),
    Dot,
}

/// A base whose kind column may be null, which `check_disjoint` refuses.
pub mod loose_base {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "loose_base")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub kind: Option<super::LooseKind>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = loose_base::Entity)]
pub enum Loose {
    Only,
}

/// Run the statements `create_disjoint` returns, in order.
async fn create<D: Disjoint>(db: &DatabaseConnection) -> Result<(), Error> {
    for statement in Schema::new().create_disjoint::<D>()? {
        db.execute(&statement.to_string(), &[]).await?;
    }
    Ok(())
}

/// The vehicles every live test but the inconsistent one reads, in key
/// order: two cars, a truck, a bike and a second truck.
struct Fleet {
    sedan: (vehicle::Model, Vehicle),
    coupe: (vehicle::Model, Vehicle),
    lorry: (vehicle::Model, Vehicle),
    roadster: (vehicle::Model, Vehicle),
    van: (vehicle::Model, Vehicle),
}

impl Fleet {
    fn all(&self) -> Vec<(vehicle::Model, Vehicle)> {
        vec![
            self.sedan.clone(),
            self.coupe.clone(),
            self.lorry.clone(),
            self.roadster.clone(),
            self.van.clone(),
        ]
    }
}

fn base(id: i32, kind: VehicleKind, name: &str) -> vehicle::Model {
    vehicle::Model {
        id,
        kind,
        name: name.to_owned(),
    }
}

async fn insert_car(db: &DatabaseConnection, name: &str, seats: i16) -> Result<i32, Error> {
    let id: i32 = db
        .query_one(
            "WITH v AS (INSERT INTO vehicle (kind, name) VALUES ('car', $1) RETURNING id) \
             INSERT INTO car (id, seats) SELECT id, $2 FROM v RETURNING id",
            &[&name, &seats],
        )
        .await?
        .get(0);
    Ok(id)
}

async fn insert_truck(
    db: &DatabaseConnection,
    name: &str,
    axles: i16,
    payload_kg: i32,
) -> Result<i32, Error> {
    let id: i32 = db
        .query_one(
            "WITH v AS (INSERT INTO vehicle (kind, name) VALUES ('truck', $1) RETURNING id) \
             INSERT INTO truck (vehicle_id, axles, payload_kg) SELECT id, $2, $3 FROM v \
             RETURNING vehicle_id",
            &[&name, &axles, &payload_kg],
        )
        .await?
        .get(0);
    Ok(id)
}

/// Create the vehicle tables and write the fleet, through raw SQL.
async fn fleet(db: &DatabaseConnection) -> Result<Fleet, Error> {
    create::<Vehicle>(db).await?;
    let sedan = insert_car(db, "sedan", 5).await?;
    let coupe = insert_car(db, "coupe", 2).await?;
    let lorry = insert_truck(db, "lorry", 3, 18_000).await?;
    let roadster: i32 = db
        .query_one(
            "INSERT INTO vehicle (kind, name) VALUES ('bike', 'roadster') RETURNING id",
            &[],
        )
        .await?
        .get(0);
    let van = insert_truck(db, "van", 2, 900).await?;
    Ok(Fleet {
        sedan: (
            base(sedan, VehicleKind::Car, "sedan"),
            Vehicle::Car(car::Model {
                id: sedan,
                seats: 5,
            }),
        ),
        coupe: (
            base(coupe, VehicleKind::Car, "coupe"),
            Vehicle::Car(car::Model {
                id: coupe,
                seats: 2,
            }),
        ),
        lorry: (
            base(lorry, VehicleKind::Truck, "lorry"),
            Vehicle::Truck(truck::Model {
                vehicle_id: lorry,
                axles: 3,
                payload_kg: 18_000,
            }),
        ),
        roadster: (base(roadster, VehicleKind::Bike, "roadster"), Vehicle::Bike),
        van: (
            base(van, VehicleKind::Truck, "van"),
            Vehicle::Truck(truck::Model {
                vehicle_id: van,
                axles: 2,
                payload_kg: 900,
            }),
        ),
    })
}

fn names(rows: &[(vehicle::Model, Vehicle)]) -> Vec<&str> {
    rows.iter().map(|(base, _)| base.name.as_str()).collect()
}

// [spec:pgorm:sem:disjoint.read+1/test]    `all`, `one` and `stream` read each base row with its kind's row as the pair, a car and a truck decoded from their tables and a bike as its unit variant, in one statement
#[pgorm_macros::test]
async fn every_kind_reads_as_its_pair_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_read_pairs").await;
    let db = ctx.db.get().await?;
    let fleet = fleet(&db).await?;

    let all = Vehicle::find()
        .order_by_asc(vehicle::Column::Id)
        .all(&db)
        .await?;
    assert_eq!(all, fleet.all());

    let lorry = Vehicle::find()
        .filter(vehicle::Column::Name.eq("lorry"))
        .one(&db)
        .await?;
    assert_eq!(lorry, fleet.lorry);

    let streamed: Vec<_> = Vehicle::find()
        .order_by_asc(vehicle::Column::Id)
        .stream(&db)
        .await?
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<_, _>>()?;
    assert_eq!(streamed, fleet.all());

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.read+1/test]    `find_by_id` reads the row of one key, a key no row has is `RecordNotFound`, and a composite key in a schema of its own reads its kind's row and its unit kind alike
#[pgorm_macros::test]
async fn find_by_id_reads_one_key_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_read_by_id").await;
    let db = ctx.db.get().await?;
    let fleet = fleet(&db).await?;

    let coupe = Vehicle::find_by_id(fleet.coupe.0.id).one(&db).await?;
    assert_eq!(coupe, fleet.coupe);
    let roadster = Vehicle::find_by_id(fleet.roadster.0.id).one(&db).await?;
    assert_eq!(roadster, fleet.roadster);
    assert_eq!(
        Vehicle::find_by_id(-1).one(&db).await,
        Err(Error::RecordNotFound)
    );

    db.execute("CREATE SCHEMA market", &[]).await?;
    create::<Listing>(&db).await?;
    db.execute(
        "WITH l AS (INSERT INTO market.listing (tenant_id, id, category, title) \
         VALUES (7, 1, 'apartment', 'loft') RETURNING tenant_id, id) \
         INSERT INTO market.flat (tenant, listing, rooms) SELECT tenant_id, id, 3 FROM l",
        &[],
    )
    .await?;
    db.execute(
        "INSERT INTO market.listing (tenant_id, id, category, title) \
         VALUES (7, 3, 'building_plot', 'field'), (8, 1, 'building_plot', 'meadow')",
        &[],
    )
    .await?;

    let loft = Listing::find_by_id((7, 1_i64)).one(&db).await?;
    assert_eq!(
        loft,
        (
            listing::Model {
                tenant_id: 7,
                id: 1,
                category: listing::ListingKind::Flat,
                title: "loft".to_owned(),
            },
            Listing::Flat(flat::Model {
                tenant: 7,
                listing: 1,
                rooms: 3,
            }),
        )
    );
    let meadow = Listing::find_by_id((8, 1_i64)).one(&db).await?;
    assert_eq!(meadow.1, Listing::BuildingPlot);
    assert_eq!(meadow.0.title, "meadow");

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.read+1/test]    `kind` and `kinds` read only base rows of those kinds, and two `kind`s read none
#[pgorm_macros::test]
async fn kind_and_kinds_narrow_by_kind_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_read_kinds").await;
    let db = ctx.db.get().await?;
    let fleet = fleet(&db).await?;
    let by_id = |select: pgorm::SelectDisjoint<Vehicle>| select.order_by_asc(vehicle::Column::Id);

    let cars = by_id(Vehicle::find().kind(VehicleKind::Car))
        .all(&db)
        .await?;
    assert_eq!(cars, [fleet.sedan.clone(), fleet.coupe.clone()]);

    let others = by_id(Vehicle::find().kinds([VehicleKind::Truck, VehicleKind::Bike]))
        .all(&db)
        .await?;
    assert_eq!(
        others,
        [
            fleet.lorry.clone(),
            fleet.roadster.clone(),
            fleet.van.clone()
        ]
    );

    let neither = Vehicle::find()
        .kind(VehicleKind::Car)
        .kind(VehicleKind::Bike)
        .all(&db)
        .await?;
    assert!(neither.is_empty(), "{neither:?}");

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.read+1/test]    a filter on a base column reads the base rows passing it; `filter_kind` reads only that kind's rows whose row passes it, a NULL-accepting condition included, where the same condition through `filter` is an ordinary WHERE over every kind's NULLs
#[pgorm_macros::test]
async fn filters_on_base_and_kind_columns_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_read_filters").await;
    let db = ctx.db.get().await?;
    let fleet = fleet(&db).await?;

    let roadster = Vehicle::find()
        .filter(vehicle::Column::Name.eq("roadster"))
        .all(&db)
        .await?;
    assert_eq!(roadster, [fleet.roadster]);

    let roomy = Vehicle::find()
        .filter_kind::<car::Model>(car::Column::Seats.gte(4))
        .all(&db)
        .await?;
    assert_eq!(roomy, [fleet.sedan]);

    let light = Vehicle::find()
        .filter_kind::<truck::Model>(truck::Column::PayloadKg.lt(1_000))
        .all(&db)
        .await?;
    assert_eq!(light, [fleet.van]);

    let axleless = Vehicle::find()
        .filter_kind::<truck::Model>(truck::Column::Axles.is_null())
        .all(&db)
        .await?;
    assert!(
        axleless.is_empty(),
        "no truck has null axles, and no other kind is read: {axleless:?}"
    );

    let unjoined = Vehicle::find()
        .filter(truck::Column::Axles.is_null())
        .order_by_asc(vehicle::Column::Id)
        .all(&db)
        .await?;
    assert_eq!(
        names(&unjoined),
        ["sedan", "coupe", "roadster"],
        "a WHERE on a kind's column sees every other kind's NULLs"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.read+1/test]    ordering, `limit` and `offset` read a window of base rows, and `count` counts the rows the read matches
#[pgorm_macros::test]
async fn ordering_and_slicing_read_base_rows_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_read_slice").await;
    let db = ctx.db.get().await?;
    let fleet = fleet(&db).await?;

    let window = Vehicle::find()
        .order_by_desc(vehicle::Column::Name)
        .limit(2)
        .offset(1)
        .all(&db)
        .await?;
    assert_eq!(window, [fleet.sedan.clone(), fleet.roadster.clone()]);

    let first_truck = Vehicle::find()
        .kind(VehicleKind::Truck)
        .order_by_desc(truck::Column::PayloadKg)
        .one(&db)
        .await?;
    assert_eq!(first_truck, fleet.lorry);

    assert_eq!(Vehicle::find().count(&db).await?, 5);
    assert_eq!(Vehicle::find().kind(VehicleKind::Car).count(&db).await?, 2);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.read+1/test]    `paginate` pages base rows as a Select's paginator does, and a keyset cursor over a base column reads forward and backward from a boundary, a kind restriction kept
#[pgorm_macros::test]
async fn pages_and_cursor_follow_the_base_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_read_pages").await;
    let db = ctx.db.get().await?;
    let fleet = fleet(&db).await?;
    const PAGE: NonZeroU64 = NonZeroU64::new(2).unwrap();

    let mut pages = Vehicle::find()
        .order_by_asc(vehicle::Column::Id)
        .paginate(&db, PAGE)?;
    assert_eq!(pages.num_items().await?, 5);
    assert_eq!(pages.num_pages().await?, 3);
    let mut read = Vec::new();
    while let Some(page) = pages.fetch_and_next().await? {
        read.push(names(&page).join(","));
    }
    assert_eq!(read, ["sedan,coupe", "lorry,roadster", "van"]);

    let mut cursor = Vehicle::find().cursor_by(vehicle::Column::Id)?;
    let after = cursor.after(fleet.coupe.0.id).first(2).all(&db).await?;
    assert_eq!(after, [fleet.lorry.clone(), fleet.roadster.clone()]);
    let before = cursor
        .after(i32::MIN)
        .before(fleet.van.0.id)
        .last(2)
        .all(&db)
        .await?;
    assert_eq!(before, [fleet.lorry.clone(), fleet.roadster.clone()]);

    let mut trucks = Vehicle::find()
        .kind(VehicleKind::Truck)
        .cursor_by(vehicle::Column::Name)?;
    let trucks = trucks.desc().first(5).all(&db).await?;
    assert_eq!(trucks, [fleet.van.clone(), fleet.lorry.clone()]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.read+1/test]    eight kinds, seven with a table, read in one statement: every kind table's `name` and `sides` and the base's `name` decode apart, and a column whose prefixed name passes 63 bytes is read under its bounded name
#[pgorm_macros::test]
async fn many_kinds_read_without_shadowing_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_read_many").await;
    let db = ctx.db.get().await?;
    create::<Shape>(&db).await?;

    let sided = [
        "triangle", "square", "pentagon", "hexagon", "heptagon", "octagon", "nonagon",
    ];
    for (at, kind) in sided.iter().enumerate() {
        let id = at as i32 + 1;
        let sides = at as i16 + 3;
        db.execute(
            &format!(
                "WITH s AS (INSERT INTO shape (id, kind, name) VALUES ($1, $2::text::shape_kind, $3)) \
                 INSERT INTO {kind} (id, name, sides) VALUES ($1, $4, $5)"
            ),
            &[&id, kind, &format!("shape {id}"), &format!("{kind} {id}"), &sides],
        )
        .await?;
    }
    db.execute(
        "UPDATE octagon SET corners_counted_one_at_a_time_all_the_way_around_the_octagon = 8",
        &[],
    )
    .await?;
    db.execute(
        "INSERT INTO shape (id, kind, name) VALUES (8, 'dot', 'shape 8')",
        &[],
    )
    .await?;

    let rows = Shape::find()
        .order_by_asc(shape::Column::Id)
        .all(&db)
        .await?;
    let bases: Vec<(i32, ShapeKind, String)> = rows
        .iter()
        .map(|(base, _)| (base.id, base.kind, base.name.clone()))
        .collect();
    assert_eq!(
        bases,
        [
            (1, ShapeKind::Triangle, "shape 1".to_owned()),
            (2, ShapeKind::Square, "shape 2".to_owned()),
            (3, ShapeKind::Pentagon, "shape 3".to_owned()),
            (4, ShapeKind::Hexagon, "shape 4".to_owned()),
            (5, ShapeKind::Heptagon, "shape 5".to_owned()),
            (6, ShapeKind::Octagon, "shape 6".to_owned()),
            (7, ShapeKind::Nonagon, "shape 7".to_owned()),
            (8, ShapeKind::Dot, "shape 8".to_owned()),
        ]
    );
    let shapes: Vec<Shape> = rows.into_iter().map(|(_, shape)| shape).collect();
    assert_eq!(
        shapes,
        [
            Shape::Triangle(triangle::Model {
                id: 1,
                name: "triangle 1".to_owned(),
                sides: 3,
            }),
            Shape::Square(square::Model {
                id: 2,
                name: "square 2".to_owned(),
                sides: 4,
            }),
            Shape::Pentagon(pentagon::Model {
                id: 3,
                name: "pentagon 3".to_owned(),
                sides: 5,
            }),
            Shape::Hexagon(hexagon::Model {
                id: 4,
                name: "hexagon 4".to_owned(),
                sides: 6,
            }),
            Shape::Heptagon(heptagon::Model {
                id: 5,
                name: "heptagon 5".to_owned(),
                sides: 7,
            }),
            Shape::Octagon(octagon::Model {
                id: 6,
                name: "octagon 6".to_owned(),
                sides: 8,
                corners_counted_one_at_a_time_all_the_way_around_the_octagon: Some(8),
            }),
            Shape::Nonagon(nonagon::Model {
                id: 7,
                name: "nonagon 7".to_owned(),
                sides: 9,
            }),
            Shape::Dot,
        ]
    );

    let sql = Shape::find().build().0;
    let bounded = pgorm::result_column_name(
        "s6_",
        "corners_counted_one_at_a_time_all_the_way_around_the_octagon",
    );
    assert_eq!(bounded.len(), 63);
    assert!(sql.contains(&format!(r#"AS "{bounded}""#)), "{sql}");
    assert!(sql.contains(r#""nonagon"."name" AS "s7_name""#), "{sql}");

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The tables of [`Vehicle`] written by hand with none of the constraints
/// `Schema::create_disjoint` writes, holding a car with no car row, a car
/// with a truck row besides its own, a bike with a truck row, and two
/// vehicles as the constraints would have them.
const LAWLESS: &str = "\
    CREATE TYPE vehicle_kind AS ENUM ('car', 'truck', 'bike'); \
    CREATE TABLE vehicle (id integer PRIMARY KEY, kind vehicle_kind NOT NULL, name text NOT NULL); \
    CREATE TABLE car (id integer PRIMARY KEY, seats smallint NOT NULL); \
    CREATE TABLE truck (vehicle_id integer PRIMARY KEY, axles smallint NOT NULL, payload_kg integer NOT NULL); \
    INSERT INTO vehicle VALUES (1, 'car', 'ghost'), (2, 'car', 'chimera'), (3, 'bike', 'roadster'), \
        (4, 'car', 'sedan'), (5, 'bike', 'hauler'); \
    INSERT INTO car VALUES (2, 4), (4, 5); \
    INSERT INTO truck VALUES (2, 2, 100), (5, 2, 300);";

fn inconsistency(outcome: Result<(vehicle::Model, Vehicle), Error>) -> String {
    match outcome {
        Err(Error::Query(RuntimeError::Internal(message))) => message,
        other => panic!("expected the row reported as inconsistent, got {other:?}"),
    }
}

// [spec:pgorm:sem:disjoint.read+1/test]    from tables written without the constraints, a car with no car row, a car with a truck row and a bike with a truck row are each an error naming the base table, the key and the kinds; `all` fails rather than dropping them, and `stream` yields every row, the consistent ones read
#[pgorm_macros::test]
async fn inconsistent_rows_are_errors_naming_them_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_read_lawless").await;
    let db = ctx.db.get().await?;
    db.batch_execute(LAWLESS).await?;

    let read = Vehicle::find()
        .order_by_asc(vehicle::Column::Id)
        .stream(&db)
        .await?
        .collect::<Vec<_>>()
        .await;
    assert_eq!(read.len(), 5, "no row is dropped");
    let mut read = read.into_iter();
    let mut next = || read.next().expect("five rows were read");

    assert_eq!(
        inconsistency(next()),
        "the `vehicle` row whose key is (`id` = 1) is of the kind `car` but has no row in \
         `car`, its kind's table: the constraints `Schema::create_disjoint` writes hold each \
         base row to exactly its own kind's table, so this schema was not created by it"
    );
    assert_eq!(
        inconsistency(next()),
        "the `vehicle` row whose key is (`id` = 2) is of the kind `car` but has a row in \
         `truck`, the table of the kind `truck`: the constraints `Schema::create_disjoint` \
         writes hold each base row to exactly its own kind's table, so this schema was not \
         created by it"
    );
    assert_eq!(
        next()?,
        (base(3, VehicleKind::Bike, "roadster"), Vehicle::Bike)
    );
    assert_eq!(
        next()?,
        (
            base(4, VehicleKind::Car, "sedan"),
            Vehicle::Car(car::Model { id: 4, seats: 5 })
        )
    );
    assert_eq!(
        inconsistency(next()),
        "the `vehicle` row whose key is (`id` = 5) is of the kind `bike` but has a row in \
         `truck`, the table of the kind `truck`: the constraints `Schema::create_disjoint` \
         writes hold each base row to exactly its own kind's table, so this schema was not \
         created by it"
    );

    let all = Vehicle::find()
        .order_by_asc(vehicle::Column::Id)
        .all(&db)
        .await;
    assert!(
        matches!(all, Err(Error::Query(_))),
        "the read fails rather than skipping rows: {all:?}"
    );
    assert_eq!(
        inconsistency(Vehicle::find_by_id(1).one(&db).await),
        inconsistency(
            Vehicle::find()
                .order_by_asc(vehicle::Column::Id)
                .one(&db)
                .await
        )
    );

    db.batch_execute(
        "INSERT INTO vehicle VALUES (6, 'truck', 'nothing'); INSERT INTO car VALUES (6, 1);",
    )
    .await?;
    assert_eq!(
        inconsistency(Vehicle::find_by_id(6).one(&db).await),
        "the `vehicle` row whose key is (`id` = 6) is of the kind `truck` but has no row in \
         `truck`, its kind's table and a row in `car`, the table of the kind `car`: the \
         constraints `Schema::create_disjoint` writes hold each base row to exactly its own \
         kind's table, so this schema was not created by it"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.read+1/test]    `check_disjoint` runs before anything is sent: a declaration it refuses is its `Error::Type` from every terminal, the paginator and the cursor, against a database holding none of the tables
#[pgorm_macros::test]
async fn a_refused_declaration_sends_nothing_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_read_refused").await;
    let db = ctx.db.get().await?;
    let refusal = match pgorm::check_disjoint::<Loose>() {
        Err(Error::Type(message)) => message,
        other => panic!("expected the declaration refused, got {other:?}"),
    };
    let refused = |outcome: Option<Error>| match outcome {
        Some(Error::Type(message)) => message,
        other => panic!("expected the declaration refused, got {other:?}"),
    };

    assert_eq!(refused(Loose::find().all(&db).await.err()), refusal);
    assert_eq!(refused(Loose::find_by_id(1).one(&db).await.err()), refusal);
    assert_eq!(refused(Loose::find().stream(&db).await.err()), refusal);
    assert_eq!(refused(Loose::find().count(&db).await.err()), refusal);
    assert_eq!(
        refused(Loose::find().paginate(&db, NonZeroU64::MIN).err()),
        refusal
    );
    assert_eq!(
        refused(Loose::find().cursor_by(loose_base::Column::Id).err()),
        refusal
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
