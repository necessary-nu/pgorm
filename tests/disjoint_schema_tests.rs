#![allow(unused_imports, dead_code)]

//! The statements `Schema::create_disjoint` returns: their text and order,
//! the names they give and those names' 63-byte bound, the declarations they
//! refuse, and, executed on a live server, the constraints that refuse every
//! state the enum does not describe.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo nextest run --test disjoint_schema_tests`

pub mod common;
pub use common::{TestContext, setup::*};

use pgorm::tests_cfg::{
    car,
    listing::{self, Listing},
    vehicle::{self, Vehicle},
};
use pgorm::{
    ConnectionTrait, DatabaseConnection, Error, KindedDisjoint, Schema, TransactionTrait,
    check_disjoint, entity::prelude::*, result_column_name,
};
use pretty_assertions::assert_eq;
use std::fmt::Debug;
use tokio_postgres::error::SqlState;

/// A kind entity keyed by one `i32` it is given.
pub mod plain {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "plain")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A base declaring the column the schema would generate for its kind `car`.
pub mod garage {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "garage")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub kind: super::GarageKind,
        pub car_id: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = garage::Entity)]
pub enum Garage {
    Car(plain::Model),
}

/// A kind entity declaring a column named as its base's kind column.
pub mod tagged {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "tagged")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub kind: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

pub mod tag_base {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "tag_base")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub kind: super::TaggingKind,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = tag_base::Entity)]
pub enum Tagging {
    Tagged(tagged::Model),
}

/// A base keyed by `b_c` and `c` with kinds valued `a` and `a_b`, whose
/// generated columns `a_` + `b_c` and `a_b_` + `c` are both `a_b_c`.
pub mod pair_base {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "pair_base")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub b_c: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub c: i32,
        pub kind: super::PairKind,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

pub mod pair_one {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "pair_one")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub x: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub y: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

pub mod pair_two {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "pair_two")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub x: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub y: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = pair_base::Entity)]
pub enum Pair {
    #[pgorm(value = "a")]
    One(pair_one::Model),
    #[pgorm(value = "a_b")]
    Two(pair_two::Model),
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
    Car(plain::Model),
}

/// A base, a kind table and a kind value long enough that every name the
/// schema composes from them runs past PostgreSQL's 63 bytes.
pub mod long_base {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "registered_vehicles_of_every_fleet_in_every_region_ever")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub kind: super::LongKind,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

pub mod long_kind {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "articulated_lorries_with_detachable_trailers_on_every_route")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

const LONG_VALUE: &str = "articulated_heavy_goods_vehicle_with_a_detachable_trailer_xy";

#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = long_base::Entity)]
pub enum Long {
    #[pgorm(value = "articulated_heavy_goods_vehicle_with_a_detachable_trailer_xy")]
    Lorry(long_kind::Model),
    Other,
}

fn rendered<D: KindedDisjoint>() -> Vec<String> {
    Schema::new()
        .create_disjoint::<D>()
        .expect("the disjoint schema is generated")
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn refusal<D: KindedDisjoint>() -> String {
    match Schema::new().create_disjoint::<D>() {
        Err(Error::Type(message)) => message,
        other => panic!("expected the schema refused as a type error, got {other:?}"),
    }
}

// [spec:pgorm:sem:disjoint.schema+1/test]    the kind type, the base with its UNIQUE key, each kind table with its generated kind column and cascading key, then one ALTER adding the base's generated key columns and keys, in that order; car's own relation to its base writes no second key
#[test]
fn vehicle_schema_renders_in_run_order() {
    assert_eq!(
        rendered::<Vehicle>(),
        [
            r#"CREATE TYPE "vehicle_kind" AS ENUM ('car', 'truck', 'bike')"#.to_owned(),
            [
                r#"CREATE TABLE "vehicle" ("#,
                r#""id" serial NOT NULL,"#,
                r#""kind" vehicle_kind NOT NULL,"#,
                r#""name" varchar NOT NULL,"#,
                r#"PRIMARY KEY ("id"),"#,
                r#"CONSTRAINT "uq-vehicle-id-kind" UNIQUE ("id", "kind")"#,
                r#")"#,
            ]
            .join(" "),
            [
                r#"CREATE TABLE "car" ("#,
                r#""id" integer NOT NULL,"#,
                r#""seats" smallint NOT NULL,"#,
                r#""kind" vehicle_kind GENERATED ALWAYS AS ('car') STORED,"#,
                r#"PRIMARY KEY ("id"),"#,
                r#"CONSTRAINT "fk-car-id-kind" FOREIGN KEY ("id", "kind")"#,
                r#"REFERENCES "vehicle" ("id", "kind")"#,
                r#"ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE"#,
                r#")"#,
            ]
            .join(" "),
            [
                r#"CREATE TABLE "truck" ("#,
                r#""vehicle_id" integer NOT NULL,"#,
                r#""axles" smallint NOT NULL,"#,
                r#""payload_kg" integer NOT NULL,"#,
                r#""kind" vehicle_kind GENERATED ALWAYS AS ('truck') STORED,"#,
                r#"PRIMARY KEY ("vehicle_id"),"#,
                r#"CONSTRAINT "fk-truck-vehicle_id-kind" FOREIGN KEY ("vehicle_id", "kind")"#,
                r#"REFERENCES "vehicle" ("id", "kind")"#,
                r#"ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE"#,
                r#")"#,
            ]
            .join(" "),
            [
                r#"ALTER TABLE "vehicle""#,
                r#"ADD COLUMN "car_id" integer"#,
                r#"GENERATED ALWAYS AS ((CASE WHEN ("kind" = 'car') THEN "id" END)) STORED,"#,
                r#"ADD COLUMN "truck_id" integer"#,
                r#"GENERATED ALWAYS AS ((CASE WHEN ("kind" = 'truck') THEN "id" END)) STORED,"#,
                r#"ADD CONSTRAINT "fk-vehicle-car_id" FOREIGN KEY ("car_id")"#,
                r#"REFERENCES "car" ("id") DEFERRABLE INITIALLY IMMEDIATE,"#,
                r#"ADD CONSTRAINT "fk-vehicle-truck_id" FOREIGN KEY ("truck_id")"#,
                r#"REFERENCES "truck" ("vehicle_id") DEFERRABLE INITIALLY IMMEDIATE"#,
            ]
            .join(" "),
        ]
    );

    // On its own, car's relation to its vehicle is a key of its own.
    let car_alone = Schema::new()
        .create_table_from_entity(car::Entity)
        .expect("the table is generated")
        .to_string();
    assert!(
        car_alone
            .contains(r#"CONSTRAINT "fk-car-id" FOREIGN KEY ("id") REFERENCES "vehicle" ("id")"#),
        "{car_alone}"
    );
}

// [spec:pgorm:sem:disjoint.schema+1/test]    a composite key, a named kind column and schema-qualified tables: each key pairs column by column, and every reference names its schema
#[test]
fn composite_schema_pairs_every_key_column() {
    assert_eq!(
        rendered::<Listing>(),
        [
            r#"CREATE TYPE "market"."listing_category" AS ENUM ('apartment', 'building_plot')"#
                .to_owned(),
            [
                r#"CREATE TABLE "market"."listing" ("#,
                r#""tenant_id" integer NOT NULL,"#,
                r#""id" bigint NOT NULL,"#,
                r#""category" market.listing_category NOT NULL,"#,
                r#""title" varchar NOT NULL,"#,
                r#"CONSTRAINT "pk-listing" PRIMARY KEY ("tenant_id", "id"),"#,
                r#"CONSTRAINT "uq-listing-tenant_id-id-category""#,
                r#"UNIQUE ("tenant_id", "id", "category")"#,
                r#")"#,
            ]
            .join(" "),
            [
                r#"CREATE TABLE "market"."flat" ("#,
                r#""tenant" integer NOT NULL,"#,
                r#""listing" bigint NOT NULL,"#,
                r#""rooms" smallint NOT NULL,"#,
                r#""category" market.listing_category GENERATED ALWAYS AS ('apartment') STORED,"#,
                r#"CONSTRAINT "pk-flat" PRIMARY KEY ("tenant", "listing"),"#,
                r#"CONSTRAINT "fk-flat-tenant-listing-category""#,
                r#"FOREIGN KEY ("tenant", "listing", "category")"#,
                r#"REFERENCES "market"."listing" ("tenant_id", "id", "category")"#,
                r#"ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE"#,
                r#")"#,
            ]
            .join(" "),
            [
                r#"ALTER TABLE "market"."listing""#,
                r#"ADD COLUMN "apartment_tenant_id" integer GENERATED ALWAYS AS"#,
                r#"((CASE WHEN ("category" = 'apartment') THEN "tenant_id" END)) STORED,"#,
                r#"ADD COLUMN "apartment_id" bigint GENERATED ALWAYS AS"#,
                r#"((CASE WHEN ("category" = 'apartment') THEN "id" END)) STORED,"#,
                r#"ADD CONSTRAINT "fk-listing-apartment_tenant_id-apartment_id""#,
                r#"FOREIGN KEY ("apartment_tenant_id", "apartment_id")"#,
                r#"REFERENCES "market"."flat" ("tenant", "listing")"#,
                r#"DEFERRABLE INITIALLY IMMEDIATE"#,
            ]
            .join(" "),
        ]
    );
}

const LONG_BASE: &str = "registered_vehicles_of_every_fleet_in_every_region_ever";
const LONG_KIND: &str = "articulated_lorries_with_detachable_trailers_on_every_route";

/// The names the long fixture's schema gives, bounded as result names are
/// under no prefix: its generated column, the base's UNIQUE key, the kind
/// table's key and the base's key to it.
fn long_names() -> [String; 4] {
    let column = result_column_name("", &format!("{LONG_VALUE}_id"));
    [
        column.clone(),
        result_column_name("", &format!("uq-{LONG_BASE}-id-kind")),
        result_column_name("", &format!("fk-{LONG_KIND}-id-kind")),
        result_column_name("", &format!("fk-{LONG_BASE}-{column}")),
    ]
}

// [spec:pgorm:sem:disjoint.schema+1/test]    a name composed past 63 bytes is bounded to 63 as result names are, with its head kept, and the statements spell it so
#[test]
fn names_past_the_bound_are_bounded() {
    let names = long_names();
    for name in &names {
        assert_eq!(name.len(), 63, "{name}");
    }
    assert!(names[0].starts_with(&LONG_VALUE[..47]));
    assert!(names[1].starts_with(&format!("uq-{LONG_BASE}")[..47]));

    let statements = rendered::<Long>().join("\n");
    for name in &names {
        assert!(
            statements.contains(&format!(r#""{name}""#)),
            "{name} in {statements}"
        );
    }
    assert!(statements.contains(&format!("('{LONG_VALUE}')")));
}

// [spec:pgorm:sem:disjoint.schema+1/test]    a generated column a base declares, a kind column a kind table declares, and one name generated twice are each refused as `Error::Type` naming both
#[test]
fn generated_name_collisions_are_refused() {
    assert_eq!(
        refusal::<Garage>(),
        "`garage`.`car_id`, the column the disjoint schema adds for the key column `id` of the \
         kind `car`, is a column `garage` declares"
    );
    assert_eq!(
        refusal::<Tagging>(),
        "`tagged`.`kind`, the kind column the disjoint schema adds to the table of the kind \
         `tagged`, is a column `tagged` declares"
    );
    assert_eq!(
        refusal::<Pair>(),
        "`pair_base`.`a_b_c`, the column the disjoint schema adds for the key column `c` of the \
         kind `a_b`, is also the one it adds for the key column `b_c` of the kind `a`"
    );
}

// [spec:pgorm:sem:disjoint.schema+1/test]    `check_disjoint` runs first, its refusal returned as the schema's
#[test]
fn declaration_is_checked_before_anything() {
    let checked = match check_disjoint::<Loose>() {
        Err(Error::Type(message)) => message,
        other => panic!("expected the declaration refused, got {other:?}"),
    };
    assert_eq!(refusal::<Loose>(), checked);
}

/// Run the statements `create_disjoint` returns, in order.
async fn create<D: KindedDisjoint>(db: &DatabaseConnection) -> Result<(), Error> {
    for statement in Schema::new().create_disjoint::<D>()? {
        db.execute(&statement.to_string(), &[]).await?;
    }
    Ok(())
}

/// Assert that PostgreSQL refused a write with a foreign key violation of
/// `constraint`.
fn assert_refused<T: Debug>(outcome: Result<T, Error>, constraint: &str) {
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

async fn count(db: &DatabaseConnection, sql: &str, id: i32) -> Result<i64, Error> {
    Ok(db.query_one(sql, &[&id]).await?.get(0))
}

const CAR_WITH_ROW: &str = "WITH v AS (INSERT INTO vehicle (kind, name) VALUES ('car', 'sedan') \
     RETURNING id) INSERT INTO car (id, seats) SELECT id, 5 FROM v RETURNING id";
const TRUCK_WITH_ROW: &str = "WITH v AS (INSERT INTO vehicle (kind, name) VALUES ('truck', \
     'lorry') RETURNING id) INSERT INTO truck (vehicle_id, axles, payload_kg) SELECT id, 3, \
     18000 FROM v RETURNING vehicle_id";
const CAR_ALONE: &str = "INSERT INTO vehicle (kind, name) VALUES ('car', 'ghost')";

// [spec:pgorm:sem:disjoint.schema+1/test]    the probe matrix: a car with its row in one statement, a bike alone, a change of kind in one statement and a cascading delete succeed; a car with no row, a truck row for a car, a vehicle with two kinds' rows, a change of kind leaving the old row and deleting a kind's row alone are refused
#[pgorm_macros::test]
async fn every_state_the_enum_omits_is_refused_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_schema").await;
    let db = ctx.db.get().await?;
    create::<Vehicle>(&db).await?;

    let car: i32 = db.query_one(CAR_WITH_ROW, &[]).await?.get(0);
    let truck: i32 = db.query_one(TRUCK_WITH_ROW, &[]).await?.get(0);
    let bike: i32 = db
        .query_one(
            "INSERT INTO vehicle (kind, name) VALUES ('bike', 'roadster') RETURNING id",
            &[],
        )
        .await?
        .get(0);

    assert_refused(db.execute(CAR_ALONE, &[]).await, "fk-vehicle-car_id");
    assert_refused(
        db.execute(
            "INSERT INTO truck (vehicle_id, axles, payload_kg) VALUES ($1, 2, 100)",
            &[&car],
        )
        .await,
        "fk-truck-vehicle_id-kind",
    );
    assert_refused(
        db.execute(
            "WITH v AS (INSERT INTO vehicle (kind, name) VALUES ('car', 'chimera') RETURNING id), \
             c AS (INSERT INTO car (id, seats) SELECT id, 2 FROM v) \
             INSERT INTO truck (vehicle_id, axles, payload_kg) SELECT id, 2, 100 FROM v",
            &[],
        )
        .await,
        "fk-truck-vehicle_id-kind",
    );
    assert_refused(
        db.execute("UPDATE vehicle SET kind = 'bike' WHERE id = $1", &[&truck])
            .await,
        "fk-truck-vehicle_id-kind",
    );
    assert_refused(
        db.execute("DELETE FROM car WHERE id = $1", &[&car]).await,
        "fk-vehicle-car_id",
    );
    let names: Vec<String> = db
        .query_all("SELECT name FROM vehicle ORDER BY id", &[])
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(
        names,
        ["sedan", "lorry", "roadster"],
        "a refusal wrote nothing"
    );

    db.execute(
        "WITH gone AS (DELETE FROM car WHERE id = $1), \
         made AS (INSERT INTO truck (vehicle_id, axles, payload_kg) VALUES ($1, 2, 900)) \
         UPDATE vehicle SET kind = 'truck' WHERE id = $1",
        &[&car],
    )
    .await?;
    db.execute(
        "WITH gone AS (DELETE FROM truck WHERE vehicle_id = $1) \
         UPDATE vehicle SET kind = 'bike' WHERE id = $1",
        &[&truck],
    )
    .await?;
    db.execute(
        "WITH made AS (INSERT INTO car (id, seats) VALUES ($1, 2)) \
         UPDATE vehicle SET kind = 'car' WHERE id = $1",
        &[&bike],
    )
    .await?;
    let kinds: Vec<(i32, String, Option<i32>, Option<i32>)> = db
        .query_all(
            "SELECT id, kind::text, car_id, truck_id FROM vehicle ORDER BY id",
            &[],
        )
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
        .collect();
    assert_eq!(
        kinds,
        [
            (car, "truck".to_owned(), None, Some(car)),
            (truck, "bike".to_owned(), None, None),
            (bike, "car".to_owned(), Some(bike), None),
        ]
    );

    db.execute("DELETE FROM vehicle WHERE id = $1", &[&bike])
        .await?;
    assert_eq!(
        count(&db, "SELECT count(*) FROM car WHERE id = $1", bike).await?,
        0,
        "the car's row went with its vehicle"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.schema+1/test]    every key is DEFERRABLE INITIALLY IMMEDIATE: inside a transaction a violation is reported by the statement causing it, and under SET CONSTRAINTS ALL DEFERRED a two-statement insert commits while one left incomplete is refused at COMMIT; the server holds the names, actions and STORED columns the statements give, and no second key for car's relation
#[pgorm_macros::test]
async fn keys_are_checked_per_statement_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_schema_txn").await;
    let mut db = ctx.db.get().await?;
    create::<Vehicle>(&db).await?;

    let outcome = db
        .transaction(async |txn| {
            assert_refused(txn.execute(CAR_ALONE, &[]).await, "fk-vehicle-car_id");
            roll_back::<()>()
        })
        .await;
    assert_rolled_back(outcome);

    let kit: i32 = db
        .transaction(async |txn| {
            txn.execute("SET CONSTRAINTS ALL DEFERRED", &[]).await?;
            let id: i32 = txn
                .query_one(
                    "INSERT INTO vehicle (kind, name) VALUES ('car', 'kit') RETURNING id",
                    &[],
                )
                .await?
                .get(0);
            txn.execute("INSERT INTO car (id, seats) VALUES ($1, 2)", &[&id])
                .await?;
            Ok::<_, Error>(id)
        })
        .await
        .expect("a deferred two-statement insert commits");
    assert_eq!(
        count(&db, "SELECT count(*) FROM car WHERE id = $1", kit).await?,
        1
    );

    let outcome = db
        .transaction(async |txn| {
            txn.execute("SET CONSTRAINTS ALL DEFERRED", &[]).await?;
            txn.execute(CAR_ALONE, &[]).await?;
            Ok::<_, Error>(())
        })
        .await;
    assert_refused(Err::<(), _>(commit_refused(outcome)), "fk-vehicle-car_id");

    let keys: Vec<(String, bool, bool, String)> = db
        .query_all(
            "SELECT conname::text, condeferrable, condeferred, confdeltype::text \
             FROM pg_constraint WHERE contype = 'f' \
             AND conrelid IN ('vehicle'::regclass, 'car'::regclass, 'truck'::regclass) \
             ORDER BY conname",
            &[],
        )
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
        .collect();
    let key = |name: &str, action: &str| (name.to_owned(), true, false, action.to_owned());
    assert_eq!(
        keys,
        [
            key("fk-car-id-kind", "c"),
            key("fk-truck-vehicle_id-kind", "c"),
            key("fk-vehicle-car_id", "a"),
            key("fk-vehicle-truck_id", "a"),
        ]
    );
    let generated: Vec<(String, String, String)> = db
        .query_all(
            "SELECT attrelid::regclass::text, attname::text, attgenerated::text \
             FROM pg_attribute WHERE attgenerated <> '' \
             AND attrelid IN ('vehicle'::regclass, 'car'::regclass, 'truck'::regclass) \
             ORDER BY 1, 2",
            &[],
        )
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();
    let stored = |table: &str, column: &str| (table.to_owned(), column.to_owned(), "s".to_owned());
    assert_eq!(
        generated,
        [
            stored("car", "kind"),
            stored("truck", "kind"),
            stored("vehicle", "car_id"),
            stored("vehicle", "truck_id"),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

const LISTING: &str = "INSERT INTO market.listing (tenant_id, id, category, title) VALUES ($1, $2, $3::text::market.listing_category, 'listed')";

// [spec:pgorm:sem:disjoint.schema+1/test]    a composite key in a schema of its own: an apartment with its flat in one statement, a building plot alone, a change of kind in one statement and a cascading delete succeed; an apartment with no flat, a flat for a building plot or for another tenant's key, and deleting a flat alone are refused
#[pgorm_macros::test]
async fn composite_keys_hold_every_row_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_schema_composite").await;
    let db = ctx.db.get().await?;
    db.execute("CREATE SCHEMA market", &[]).await?;
    create::<Listing>(&db).await?;

    db.execute(
        "WITH l AS (INSERT INTO market.listing (tenant_id, id, category, title) \
         VALUES (7, 1, 'apartment', 'loft') RETURNING tenant_id, id) \
         INSERT INTO market.flat (tenant, listing, rooms) SELECT tenant_id, id, 3 FROM l",
        &[],
    )
    .await?;
    db.execute(LISTING, &[&7_i32, &3_i64, &"building_plot"])
        .await?;

    assert_refused(
        db.execute(LISTING, &[&7_i32, &2_i64, &"apartment"]).await,
        "fk-listing-apartment_tenant_id-apartment_id",
    );
    assert_refused(
        db.execute(
            "INSERT INTO market.flat (tenant, listing, rooms) VALUES (7, 3, 2)",
            &[],
        )
        .await,
        "fk-flat-tenant-listing-category",
    );
    assert_refused(
        db.execute(
            "INSERT INTO market.flat (tenant, listing, rooms) VALUES (9, 1, 2)",
            &[],
        )
        .await,
        "fk-flat-tenant-listing-category",
    );
    assert_refused(
        db.execute(
            "DELETE FROM market.flat WHERE tenant = 7 AND listing = 1",
            &[],
        )
        .await,
        "fk-listing-apartment_tenant_id-apartment_id",
    );

    db.execute(
        "WITH gone AS (DELETE FROM market.flat WHERE tenant = 7 AND listing = 1) \
         UPDATE market.listing SET category = 'building_plot' WHERE tenant_id = 7 AND id = 1",
        &[],
    )
    .await?;
    db.execute(
        "WITH made AS (INSERT INTO market.flat (tenant, listing, rooms) VALUES (7, 3, 1)) \
         UPDATE market.listing SET category = 'apartment' WHERE tenant_id = 7 AND id = 3",
        &[],
    )
    .await?;
    db.execute(
        "DELETE FROM market.listing WHERE tenant_id = 7 AND id = 3",
        &[],
    )
    .await?;
    let flats: i64 = db
        .query_one("SELECT count(*) FROM market.flat", &[])
        .await?
        .get(0);
    assert_eq!(flats, 0, "the flat went with its listing");
    let listings: Vec<(i64, String)> = db
        .query_all(
            "SELECT id, category::text FROM market.listing ORDER BY id",
            &[],
        )
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(listings, [(1, "building_plot".to_owned())]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.schema+1/test]    names bounded to 63 bytes are the server's names: it creates every constraint and column under exactly the name the statements give, none truncated
#[pgorm_macros::test]
async fn bounded_names_are_the_servers_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_schema_names").await;
    let db = ctx.db.get().await?;
    create::<Long>(&db).await?;

    let [column, unique, kind_key, base_key] = long_names();
    let mut constraints: Vec<String> = db
        .query_all(
            "SELECT conname::text FROM pg_constraint WHERE contype IN ('f', 'u') \
             AND connamespace = 'public'::regnamespace ORDER BY conname",
            &[],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    constraints.sort();
    let mut expected = vec![unique, kind_key, base_key];
    expected.sort();
    assert_eq!(constraints, expected);
    let columns: Vec<String> = db
        .query_all(
            "SELECT attname::text FROM pg_attribute WHERE attrelid = $1::text::regclass \
             AND attgenerated <> ''",
            &[&format!("\"{LONG_BASE}\"")],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(columns, [column]);

    drop(db);
    ctx.delete().await;
    Ok(())
}
