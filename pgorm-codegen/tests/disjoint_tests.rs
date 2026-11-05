//! A disjoint entity's tables — a base and its kinds' tables, constrained as
//! `Schema::create_disjoint` writes them — generate as the base's entity
//! holding a `#[derive(Disjoint)]` enum and each kind's entity, from DDL text
//! and from statements built in code alike; a set missing any part of the
//! shape, or with any part different, generates as the ordinary tables it is.
//!
//! The files the writer is held to here are compiled, and the schema they
//! were generated from created and written through against a live server, at
//! the workspace root (`tests/codegen_disjoint_tests.rs`), whose module tree
//! they are. The ones generated under serde are compiled here, in
//! `disjoint_serde`, and so are the ones whose variants end with their enum's
//! name, in `disjoint_variant_names`, under the workspace's clippy.

mod common;
mod disjoint_serde;
mod disjoint_variant_names;

use common::runtime_name as n;
use common::*;
use pgorm::{ActiveEnum, Disjoint};
use pgorm_codegen::{TableIdent, UnkindedDisjoint, WithSerde};
use pgorm_query::{
    ColumnDef, ColumnType, Deferrability, Expr, ForeignKey, ForeignKeyAction, GeneratedKind, Table,
    TableCreateStatement, TableKey,
};

const SCHEMA: &str = include_str!("../../tests/codegen_disjoint/schema.sql");

/// The vehicle's statements: the schema up to the listing's.
fn vehicle_schema() -> &'static str {
    SCHEMA
        .split("CREATE SCHEMA")
        .next()
        .expect("the schema opens with the vehicle's statements")
}

// [spec:pgorm:sem:codegen.entity.disjoint+3/test]    the statements `create_disjoint` writes for the vehicle and the composite listing, read as DDL text, generate each base's entity with its `#[derive(Disjoint)]` enum and each kind's entity, with no field for a bookkeeping column, a relation from each kind's key to its base and the base's to each kind, `kind`, `enum_name` and `schema_name` where they differ from the derive's defaults, and no `pgorm_active_enums.rs` for the kind types the derives declare
// [spec:pgorm:sem:codegen.ddl.objects+12/test]    the ALTER TABLE adding the bases' generated key columns and keys folds into the base
#[test]
fn disjoint_tables_generate_one_disjoint_entity() {
    let generated = from_sql_with(SCHEMA, Opts::default());
    assert_eq!(
        generated.names(),
        [
            "car.rs",
            "flat.rs",
            "listing.rs",
            "truck.rs",
            "vehicle.rs",
            "mod.rs",
            "prelude.rs"
        ]
    );
    for (file, fixture) in [
        ("vehicle.rs", "../../tests/codegen_disjoint/vehicle.rs"),
        ("car.rs", "../../tests/codegen_disjoint/car.rs"),
        ("truck.rs", "../../tests/codegen_disjoint/truck.rs"),
        ("listing.rs", "../../tests/codegen_disjoint/listing.rs"),
        ("flat.rs", "../../tests/codegen_disjoint/flat.rs"),
    ] {
        generated.holds(file, fixture);
    }
    assert_not_contains(generated.file("mod.rs"), "pub mod pgorm_active_enums;");
}

/// The vehicle's tables as a schema described in code holds them, each whole:
/// the base with the key columns and keys the `ALTER TABLE` adds.
fn described_vehicle() -> Vec<TableCreateStatement> {
    let kind_type = || ColumnType::Enum {
        name: n("vehicle_kind"),
        schema: None,
        variants: vec![n("car"), n("truck"), n("bike")],
    };
    let generated_key = |name: &str, value: &str| {
        ColumnDef::new(n(name))
            .integer()
            .generated(
                Expr::case(Expr::col(n("kind")).eq(value), Expr::col(n("id"))),
                GeneratedKind::Stored,
            )
            .to_owned()
    };
    let deferrable = Deferrability::DeferrableInitiallyImmediate;
    let vehicle = Table::create(n("vehicle"))
        .col(serial("id"))
        .col(
            ColumnDef::new_with_type(n("kind"), kind_type())
                .not_null()
                .to_owned(),
        )
        .col(col("name").string().not_null().to_owned())
        .col(generated_key("car_id", "car"))
        .col(generated_key("truck_id", "truck"))
        .primary_key(n("id"))
        .unique(
            TableKey::new(n("id"))
                .col(n("kind"))
                .name(n("uq-vehicle-id-kind")),
        )
        .foreign_key(
            ForeignKey::create(n("vehicle"), n("car_id"), n("car"), n("id"))
                .name(n("fk-vehicle-car_id"))
                .deferrability(deferrable)
                .to_owned(),
        )
        .foreign_key(
            ForeignKey::create(n("vehicle"), n("truck_id"), n("truck"), n("vehicle_id"))
                .name(n("fk-vehicle-truck_id"))
                .deferrability(deferrable)
                .to_owned(),
        )
        .to_owned();
    let kind = |table: &str, key: &str, value: &str, columns: Vec<ColumnDef>| {
        let mut create = Table::create(n(table));
        create.col(col(key).integer().not_null().to_owned());
        for column in columns {
            create.col(column);
        }
        create
            .col(
                ColumnDef::new_with_type(n("kind"), kind_type())
                    .generated(Expr::val(value), GeneratedKind::Stored)
                    .to_owned(),
            )
            .primary_key(n(key))
            .foreign_key(
                ForeignKey::create(n(table), n(key), n("vehicle"), n("id"))
                    .col(n("kind"), n("kind"))
                    .name(n(&format!("fk-{table}-{key}-kind")))
                    .on_delete(ForeignKeyAction::Cascade)
                    .deferrability(deferrable)
                    .to_owned(),
            )
            .to_owned()
    };
    vec![
        vehicle,
        kind(
            "car",
            "id",
            "car",
            vec![col("seats").small_integer().not_null().to_owned()],
        ),
        kind(
            "truck",
            "vehicle_id",
            "truck",
            vec![
                col("axles").small_integer().not_null().to_owned(),
                col("payload_kg").integer().not_null().to_owned(),
            ],
        ),
    ]
}

// [spec:pgorm:sem:codegen.entity.disjoint+3/test]    a schema described in code, each table whole as it stands in the database, generates as the same DDL text does
#[test]
fn described_tables_generate_as_ddl_text_does() {
    let described = generate(described_vehicle(), Opts::default());
    assert_eq!(
        described.files,
        from_sql_with(vehicle_schema(), Opts::default()).files
    );
    described.holds("vehicle.rs", "../../tests/codegen_disjoint/vehicle.rs");
}

/// The fields of one generated `Model`, in order.
fn fields(file: &str) -> Vec<String> {
    let file = norm(file);
    let model = file
        .split_once("pub struct Model {")
        .and_then(|(_, rest)| rest.split_once(" } "))
        .map(|(model, _)| model)
        .expect("the file declares a Model");
    model
        .split(" pub ")
        .skip(1)
        .filter_map(|field| field.split_once(" : ").map(|(name, _)| name.to_owned()))
        .collect()
}

// [spec:pgorm:sem:codegen.entity.disjoint+3/test]    a set with any part of the shape missing or different — the base's UNIQUE pair, a kind's key's columns, action or deferrability, a kind column's expression, STORED or nullability, a kind's key the database fills or a key of its own to the base, a base key column's expression, STORED or key, and the base's kind column's nullability — generates as the ordinary entities it is, every column kept and every generated column generated
#[test]
fn near_misses_generate_ordinary_entities() {
    let car_key = r#"CONSTRAINT "fk-car-id-kind" FOREIGN KEY ("id", "kind") REFERENCES "vehicle" ("id", "kind") ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE"#;
    let car_kind = r#""kind" vehicle_kind GENERATED ALWAYS AS ('car') STORED"#;
    let car_id = r#""car_id" integer GENERATED ALWAYS AS ((CASE WHEN ("kind" = 'car') THEN "id" END)) STORED"#;
    let base_key = r#"ADD CONSTRAINT "fk-vehicle-car_id" FOREIGN KEY ("car_id") REFERENCES "car" ("id") DEFERRABLE INITIALLY IMMEDIATE"#;
    let unique = r#"CONSTRAINT "uq-vehicle-id-kind" UNIQUE ("id", "kind")"#;
    let car_key_with = |tail: &str| {
        format!(
            r#"CONSTRAINT "fk-car-id-kind" FOREIGN KEY ("id", "kind") REFERENCES "vehicle" ("id", "kind") {tail}"#
        )
    };
    let near_misses: Vec<(&str, String, String)> = vec![
        (
            "the base's UNIQUE pair missing",
            format!(", {unique}"),
            String::new(),
        ),
        (
            "the UNIQUE pair over its columns in another order",
            unique.to_owned(),
            r#"CONSTRAINT "uq-vehicle-id-kind" UNIQUE ("kind", "id")"#.to_owned(),
        ),
        (
            "the UNIQUE pair NULLS NOT DISTINCT",
            unique.to_owned(),
            r#"CONSTRAINT "uq-vehicle-id-kind" UNIQUE NULLS NOT DISTINCT ("id", "kind")"#
                .to_owned(),
        ),
        (
            "a kind's key not cascading",
            car_key.to_owned(),
            car_key_with("DEFERRABLE INITIALLY IMMEDIATE"),
        ),
        (
            "a kind's key cascading updates",
            car_key.to_owned(),
            car_key_with("ON UPDATE CASCADE ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE"),
        ),
        (
            "a kind's key not deferrable",
            car_key.to_owned(),
            car_key_with("ON DELETE CASCADE"),
        ),
        (
            "a kind's key initially deferred",
            car_key.to_owned(),
            car_key_with("ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED"),
        ),
        (
            "a kind's key over its columns in another order",
            car_key.to_owned(),
            r#"CONSTRAINT "fk-car-id-kind" FOREIGN KEY ("kind", "id") REFERENCES "vehicle" ("kind", "id") ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE"#.to_owned(),
        ),
        (
            "a kind's key missing",
            format!(", {car_key}"),
            String::new(),
        ),
        (
            "a kind column not generated",
            car_kind.to_owned(),
            r#""kind" vehicle_kind"#.to_owned(),
        ),
        (
            "a kind column VIRTUAL",
            car_kind.to_owned(),
            r#""kind" vehicle_kind GENERATED ALWAYS AS ('car') VIRTUAL"#.to_owned(),
        ),
        (
            "a kind column of another value",
            car_kind.to_owned(),
            r#""kind" vehicle_kind GENERATED ALWAYS AS ('bike') STORED"#.to_owned(),
        ),
        (
            "a kind column NOT NULL",
            car_kind.to_owned(),
            format!("{car_kind} NOT NULL"),
        ),
        (
            "a kind's key drawn from a sequence",
            r#""id" integer NOT NULL, "seats""#.to_owned(),
            r#""id" serial NOT NULL, "seats""#.to_owned(),
        ),
        (
            "a kind with a key of its own to the base",
            car_key.to_owned(),
            format!(r#"{car_key}, FOREIGN KEY ("id") REFERENCES "vehicle" ("id")"#),
        ),
        (
            "a base key column VIRTUAL",
            car_id.to_owned(),
            car_id.replace(" STORED", " VIRTUAL"),
        ),
        (
            "a base key column with an ELSE",
            car_id.to_owned(),
            car_id.replace(r#"THEN "id" END"#, r#"THEN "id" ELSE 0 END"#),
        ),
        (
            "a base key column testing another value",
            car_id.to_owned(),
            car_id.replace("'car'", "'truck'"),
        ),
        (
            "a base key column NOT NULL",
            car_id.to_owned(),
            format!("{car_id} NOT NULL"),
        ),
        (
            "a base key's key missing",
            format!("{base_key}, "),
            String::new(),
        ),
        (
            "a base key's key not deferrable",
            base_key.to_owned(),
            base_key.replace(" DEFERRABLE INITIALLY IMMEDIATE", ""),
        ),
        (
            "a base key's key cascading",
            base_key.to_owned(),
            base_key.replace(" DEFERRABLE", " ON DELETE CASCADE DEFERRABLE"),
        ),
        (
            "a base key's key to another table",
            base_key.to_owned(),
            base_key.replace(r#""car" ("id")"#, r#""truck" ("vehicle_id")"#),
        ),
        (
            "the base's kind column nullable",
            r#""kind" vehicle_kind NOT NULL"#.to_owned(),
            r#""kind" vehicle_kind"#.to_owned(),
        ),
    ];
    let generated_fields = [
        (
            "vehicle.rs",
            vec!["id", "kind", "name", "car_id", "truck_id"],
        ),
        ("car.rs", vec!["id", "seats", "kind"]),
        (
            "truck.rs",
            vec!["vehicle_id", "axles", "payload_kg", "kind"],
        ),
    ];
    for (what, from, to) in near_misses {
        let sql = vehicle_schema().replacen(&from, &to, 1);
        assert_ne!(sql, vehicle_schema(), "{what}: the schema is changed");
        let generated = from_sql_with(&sql, Opts::default());
        for (file, expected) in &generated_fields {
            let content = generated.file(file);
            assert_not_contains(content, "Disjoint");
            assert_eq!(fields(content), *expected, "{what}: {file}");
        }
        let generated_attrs = |file: &str| norm(generated.file(file)).matches("generated_").count();
        let kind_generated = usize::from(what != "a kind column not generated");
        assert_eq!(generated_attrs("vehicle.rs"), 2, "{what}");
        assert_eq!(generated_attrs("car.rs"), kind_generated, "{what}");
        assert_eq!(generated_attrs("truck.rs"), 1, "{what}");
        assert_contains(
            generated.file("pgorm_active_enums.rs"),
            "pub enum VehicleKind",
        );
    }

    let not_deferrable = from_sql_with(
        &vehicle_schema().replacen(
            base_key,
            &base_key.replace(" DEFERRABLE INITIALLY IMMEDIATE", ""),
            1,
        ),
        Opts::default(),
    );
    assert_contains(
        not_deferrable.file("vehicle.rs"),
        r#"#[pgorm(generated_stored = Expr::case(Expr::col(Column::Kind).eq(Expr::val("car")), Expr::col(Column::Id)))]
        pub car_id: Option<i32>,"#,
    );
    assert_contains(
        not_deferrable.file("car.rs"),
        r#"use super::pgorm_active_enums::VehicleKind;"#,
    );
    assert_contains(
        not_deferrable.file("car.rs"),
        r#"#[pgorm(generated_stored = Expr::val("car"))]
        pub kind: Option<VehicleKind>,"#,
    );
}

// [spec:pgorm:sem:codegen.entity.disjoint+3/test]    a column of a kind type elsewhere is typed as the derive's kind type, imported from the base's module, and an enum the kind type is not is generated as ever
// [spec:pgorm:sem:codegen.entity.imports+5/test]
#[test]
fn a_kind_type_elsewhere_is_the_derives() {
    let sql = format!(
        "{} CREATE TYPE colour AS ENUM ('red', 'blue'); \
         CREATE TABLE fleet (id int PRIMARY KEY, kinds vehicle_kind[] NOT NULL, \
         preferred vehicle_kind, livery colour);",
        vehicle_schema()
    );
    let generated = from_sql_with(&sql, Opts::default());
    let fleet = generated.file("fleet.rs");
    assert_contains(fleet, "use super::vehicle::VehicleKind;");
    assert_contains(fleet, "use super::pgorm_active_enums::Colour;");
    assert_contains(
        fleet,
        "pub kinds: Vec<VehicleKind>, pub preferred: Option<VehicleKind>,",
    );
    let enums = generated.file("pgorm_active_enums.rs");
    assert_contains(enums, "pub enum Colour");
    assert_not_contains(enums, "VehicleKind");
    assert_not_contains(generated.file("vehicle.rs"), "use super::");
}

// [spec:pgorm:sem:codegen.entity.disjoint+3/test]    every variant is named as an enum's values are, carrying `value` where its value is not its name in snake_case, and the enum `enum_name` where its kind type is not named for it; a base all of whose kinds are unit kinds is a disjoint entity too
#[test]
fn variants_say_what_differs_from_the_derive() {
    let generated = from_sql_with(
        "CREATE TYPE gadget_type AS ENUM ('Phone', 'smart watch', 'tablet'); \
         CREATE TABLE gadget (id int PRIMARY KEY, kind gadget_type NOT NULL, \
         UNIQUE (id, kind));",
        Opts::default(),
    );
    assert_contains(
        generated.file("gadget.rs"),
        r#"#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
        #[pgorm(base = Entity, enum_name = "gadget_type")]
        pub enum Gadget {
            #[pgorm(value = "Phone")]
            Phone,
            #[pgorm(value = "smart watch")]
            SmartWatch,
            Tablet,
        }"#,
    );
    assert_contains(generated.file("gadget.rs"), "pub kind: GadgetKind,");
    assert!(!generated.has("pgorm_active_enums.rs"));
}

// [spec:pgorm:sem:codegen.entity.disjoint+3/test]    a base with two kind columns each completing the shape, a kind type two bases share, and a disjoint enum, kind type or active model named as an item of the base's module, a type it imports or a name its code means otherwise, are refused naming both
#[test]
fn disjoint_names_that_clash_are_refused() {
    let unit = |table: &str, columns: &str| {
        format!(
            "CREATE TABLE {table} (id int PRIMARY KEY, kind {table}_kind NOT NULL{columns}, \
             UNIQUE (id, kind));"
        )
    };
    assert_eq!(
        refused(
            "CREATE TYPE a AS ENUM ('x'); CREATE TYPE b AS ENUM ('y'); \
             CREATE TABLE t (id int PRIMARY KEY, ka a NOT NULL, kb b NOT NULL, \
             UNIQUE (id, ka), UNIQUE (id, kb));"
        ),
        "table `t`: columns `ka` and `kb` each make it the base of a disjoint entity, whose \
         enum is named for the table, so only one can be generated"
    );
    assert_eq!(
        refused(
            "CREATE TYPE shared AS ENUM ('x'); \
             CREATE TABLE t (id int PRIMARY KEY, kind shared NOT NULL, UNIQUE (id, kind)); \
             CREATE TABLE u (id int PRIMARY KEY, kind shared NOT NULL, UNIQUE (id, kind));"
        ),
        "enum `shared` is the kind type of the disjoint entities of tables `t` and `u`; each \
         one's derive declares a kind type of its own"
    );
    assert_eq!(
        refused(&format!(
            "CREATE TYPE model_kind AS ENUM ('x'); {}",
            unit("model", "")
        )),
        "table `model`: its disjoint entity's enum `Model` is named as the entity's own `Model`"
    );
    assert_eq!(
        refused(&format!(
            "CREATE TYPE string_kind AS ENUM ('x'); {}",
            unit("string", ", label text NOT NULL")
        )),
        "table `string`: its disjoint entity's enum `String` is named as the `String` its \
         module's code names"
    );
    assert_eq!(
        refused(&format!(
            "CREATE TYPE tint_kind AS ENUM ('x'); CREATE TYPE tint AS ENUM ('red'); {}",
            unit("tint", ", shade tint")
        )),
        "table `tint`: its disjoint entity's enum `Tint` is named as the type enum `tint` \
         generates"
    );
}

// [spec:pgorm:sem:codegen.entity.disjoint+3/test]    a disjoint enum named as a derive the module's code lists, a base table named `debug`, is generated: a `derive(..)` list's names are macros, which no item of the module shadows
#[test]
fn a_base_named_like_a_derive_is_generated() {
    let generated = from_sql_with(
        "CREATE TYPE debug_kind AS ENUM ('trace', 'span'); \
         CREATE TABLE debug (id int PRIMARY KEY, kind debug_kind NOT NULL, UNIQUE (id, kind));",
        Opts::default(),
    );
    assert_contains(
        generated.file("debug.rs"),
        "#[derive(Clone, Debug, PartialEq, Eq, Disjoint)] #[pgorm(base = Entity)] pub enum Debug",
    );
}

/// A row of each kind of the serde-generated vehicle, as its base row and
/// its disjoint value.
fn serde_vehicles() -> Vec<(
    disjoint_serde::vehicle::Model,
    disjoint_serde::vehicle::Vehicle,
)> {
    use disjoint_serde::{car, truck, vehicle};
    let base = |id: i32, kind: vehicle::VehicleKind| vehicle::Model {
        id,
        kind,
        name: format!("vehicle {id}"),
    };
    vec![
        (
            base(1, vehicle::VehicleKind::Car),
            vehicle::Vehicle::Car(car::Model { id: 1, seats: 5 }),
        ),
        (
            base(2, vehicle::VehicleKind::Truck),
            vehicle::Vehicle::Truck(truck::Model {
                vehicle_id: 2,
                axles: 3,
                payload_kg: 18_000,
            }),
        ),
        (base(3, vehicle::VehicleKind::Bike), vehicle::Vehicle::Bike),
    ]
}

// [spec:pgorm:sem:codegen.entity.disjoint+3/test]    under serde the disjoint enum derives serde as the `Model`s it holds do, and its kind type through `kind_attrs`, so the base's `Model` serializes with its kind field
#[test]
fn disjoint_entities_serialize_under_serde() {
    let generated = from_sql_with(
        vehicle_schema(),
        Opts {
            with_serde: WithSerde::Both,
            ..Default::default()
        },
    );
    for (file, fixture) in [
        ("vehicle.rs", "disjoint_serde/vehicle.rs"),
        ("car.rs", "disjoint_serde/car.rs"),
        ("truck.rs", "disjoint_serde/truck.rs"),
    ] {
        generated.holds(file, fixture);
    }

    for (base, value) in serde_vehicles() {
        let json = serde_json::to_value((&base, &value)).expect("the pair serializes");
        let read: (
            disjoint_serde::vehicle::Model,
            disjoint_serde::vehicle::Vehicle,
        ) = serde_json::from_value(json).expect("the pair deserializes");
        assert_eq!(read, (base.clone(), value.clone()));
        assert_eq!(base.kind, value.kind());
        assert_eq!(
            disjoint_serde::vehicle::VehicleKind::name().to_string(),
            "vehicle_kind"
        );
    }
}

// [spec:pgorm:sem:codegen.entity.disjoint+3/test]    a name the base's code reaches through a path, as a junction's `Relation` variant named for the base, is no clash
#[test]
fn a_junction_to_the_base_is_no_clash() {
    let sql = format!(
        "{} CREATE TABLE garage (id int PRIMARY KEY); \
         CREATE TABLE vehicle_garage (vehicle_id int REFERENCES vehicle (id), \
         garage_id int REFERENCES garage (id), PRIMARY KEY (vehicle_id, garage_id));",
        vehicle_schema()
    );
    let generated = from_sql_with(&sql, Opts::default());
    let vehicle = generated.file("vehicle.rs");
    assert_contains(
        vehicle,
        "super::vehicle_garage::Relation::Vehicle.def().rev()",
    );
    assert_contains(vehicle, "#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]");
}

const NEAR_MISS: &str = include_str!("../../tests/codegen_near_miss/schema.sql");

// [spec:pgorm:sem:codegen.entity.disjoint+3/test]    a near miss keeps its tables' composite unique keys: the base's UNIQUE pair the kind's foreign key references is generated as the base entity's `unique(..)` beside its other, so the entities create the tables they were read from (live in `tests/codegen_near_miss_tests.rs`)
// [spec:pgorm:sem:codegen.entity.compact.attrs+14/test]
#[test]
fn near_miss_keeps_its_unique_keys() {
    let generated = from_sql_with(NEAR_MISS, Opts::default());
    for (file, fixture) in [
        ("shelf.rs", "../../tests/codegen_near_miss/shelf.rs"),
        ("book.rs", "../../tests/codegen_near_miss/book.rs"),
        (
            "pgorm_active_enums.rs",
            "../../tests/codegen_near_miss/pgorm_active_enums.rs",
        ),
    ] {
        generated.holds(file, fixture);
    }
}

/// The store's tables, named as an unkinded disjoint entity with a unit kind.
fn stores() -> UnkindedDisjoint {
    let table = |name: &str| TableIdent {
        table: name.to_owned(),
        schema: None,
    };
    UnkindedDisjoint {
        base: table("store"),
        kinds: vec![table("web_store"), table("pop_up_store")],
        unit: Some("closed_store".to_owned()),
    }
}

// [spec:pgorm:sem:codegen.entity.disjoint+3/test]    a kinded enum every variant of which ends with its name, `Shop::WebShop`, carries the allow of clippy's `enum_variant_names`, and compiles clippy-clean in a module that is not exported
// [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1/test]    and so does an unkinded one, `Store::WebStore`
#[test]
fn variants_named_as_their_enum_compile_clean() {
    use disjoint_variant_names::{pop_up_shop, shop, store, web_shop, web_store};
    let generated = from_sql_with(
        include_str!("disjoint_variant_names/schema.sql"),
        Opts {
            unkinded_disjoint: vec![stores()],
            ..Default::default()
        },
    );
    for (file, fixture) in [
        ("shop.rs", "disjoint_variant_names/shop.rs"),
        ("web_shop.rs", "disjoint_variant_names/web_shop.rs"),
        ("pop_up_shop.rs", "disjoint_variant_names/pop_up_shop.rs"),
        ("store.rs", "disjoint_variant_names/store.rs"),
        ("web_store.rs", "disjoint_variant_names/web_store.rs"),
        ("pop_up_store.rs", "disjoint_variant_names/pop_up_store.rs"),
    ] {
        generated.holds(file, fixture);
    }

    let site = shop::Shop::WebShop(web_shop::Model {
        id: 1,
        url: "https://example.com".to_owned(),
    });
    assert_eq!(site.kind(), shop::ShopKind::WebShop);
    assert_eq!(
        shop::Shop::PopUpShop(pop_up_shop::Model {
            id: 2,
            site: "market".to_owned(),
        })
        .kind(),
        shop::ShopKind::PopUpShop
    );
    assert_eq!(shop::Shop::ClosedShop.kind(), shop::ShopKind::ClosedShop);
    assert_eq!(
        shop::ShopKind::values(),
        ["web_shop", "pop_up_shop", "closed_shop"]
    );
    let online = store::Store::WebStore(web_store::Model {
        store_id: 3,
        url: "https://example.org".to_owned(),
    });
    assert_eq!(online.kind(), store::StoreKind::WebStore);
    assert_eq!(
        store::Store::ClosedStore.kind(),
        store::StoreKind::ClosedStore
    );
    pgorm::check_disjoint::<shop::Shop>().expect("the generated shop is sound");
    pgorm::check_disjoint::<store::Store>().expect("the generated store is sound");
}
