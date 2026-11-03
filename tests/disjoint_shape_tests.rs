#![allow(unused_imports, dead_code)]

//! A disjoint entity described at run time, as a language binding describes
//! one from its models: the statements its shape builds are the ones the
//! typed `Vehicle` builds, what the types would hold is refused as it is
//! described, and its schema, writes and reads run on a live server, each row
//! judged by its kind.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo nextest run --test disjoint_shape_tests`

pub mod common;
pub use common::{TestContext, setup::*};

use pgorm::pgorm_query::{
    ColumnType, Name, SelectStatement, StringLen, TableName, TypeName, Value, Values,
};
use pgorm::tests_cfg::{
    car, truck,
    vehicle::{self, Vehicle, VehicleActiveModel, VehicleKind},
};
use pgorm::types::ToSql;
use pgorm::{
    ActiveValue, ColumnTypeTrait, ConnectionTrait, DatabaseConnection, Disjoint, DisjointColumn,
    DisjointJudged, DisjointShape, DisjointTable, DisjointValues, Error, QueryResult, QueryTrait,
    ValueHolder, count_statement, entity::prelude::*, page_statement,
};
use pretty_assertions::assert_eq;
use std::num::NonZeroU64;

fn table(name: &str, columns: Vec<DisjointColumn>, key: &[&str]) -> DisjointTable {
    DisjointTable::new(TableName::Table(Name::runtime(name)), columns, key)
        .expect("the table is described")
}

fn vehicle_kind() -> ColumnType {
    ColumnType::Enum {
        name: Name::runtime("vehicle_kind"),
        schema: None,
        variants: ["car", "truck", "bike"].map(Name::runtime).to_vec(),
    }
}

fn base() -> DisjointTable {
    table(
        "vehicle",
        vec![
            DisjointColumn::new("id", ColumnType::Integer.def().identity_by_default()),
            DisjointColumn::new("kind", vehicle_kind().def()),
            DisjointColumn::new("name", ColumnType::String(StringLen::None).def()),
        ],
        &["id"],
    )
}

fn car() -> DisjointTable {
    table(
        "car",
        vec![
            DisjointColumn::new("id", ColumnType::Integer.def()),
            DisjointColumn::new("seats", ColumnType::SmallInteger.def()),
        ],
        &["id"],
    )
}

fn truck() -> DisjointTable {
    table(
        "truck",
        vec![
            DisjointColumn::new("vehicle_id", ColumnType::Integer.def()),
            DisjointColumn::new("axles", ColumnType::SmallInteger.def()),
            DisjointColumn::new("payload_kg", ColumnType::Integer.def()),
        ],
        &["vehicle_id"],
    )
}

fn kind_type() -> TypeName {
    TypeName::new(Name::runtime("vehicle_kind"))
}

/// `Vehicle` described at run time.
fn vehicles() -> DisjointShape {
    DisjointShape::new(
        base(),
        "kind",
        kind_type(),
        vec![
            ("car".to_owned(), Some(car())),
            ("truck".to_owned(), Some(truck())),
            ("bike".to_owned(), None),
        ],
    )
    .expect("the shape is described")
}

fn set(name: &str, value: impl Into<Value>) -> (String, ActiveValue<Value>) {
    (name.to_owned(), ActiveValue::Set(value.into()))
}

fn values(table: &DisjointTable, given: Vec<(String, ActiveValue<Value>)>) -> DisjointValues {
    table.values(given).expect("the values are the table's")
}

fn with(query: SelectStatement, condition: pgorm::pgorm_query::SimpleExpr) -> SelectStatement {
    let mut query = query;
    query.and_where(condition);
    query
}

// [spec:pgorm:req:bindings.disjoint+1/test]    a shape described at run time builds the statements the typed declaration of the same tables builds: the read, its narrowing by kind, kinds and a kind's condition, and each write
#[test]
fn a_described_shape_builds_the_typed_statements() {
    let shape = vehicles();
    assert_eq!(shape.find().build(), Vehicle::find().build());
    assert_eq!(
        with(shape.find(), shape.kind_is("car").expect("car is a kind")).build(),
        Vehicle::find().kind(VehicleKind::Car).build()
    );
    assert_eq!(
        with(
            shape.find(),
            shape.kind_in(&["truck", "bike"]).expect("both are kinds")
        )
        .build(),
        Vehicle::find()
            .kinds([VehicleKind::Truck, VehicleKind::Bike])
            .build()
    );
    let mut narrowed = shape.find();
    narrowed.cond_where(
        shape
            .filter_kind("car", car::Column::Seats.gte(4))
            .expect("car is a kind"),
    );
    assert_eq!(
        narrowed.build(),
        Vehicle::find()
            .filter_kind::<car::Model>(car::Column::Seats.gte(4))
            .build()
    );

    let base_values = values(&base(), vec![set("name", "mini")]);
    let car_values = values(&car(), vec![set("seats", 4_i16)]);
    assert_eq!(
        shape
            .insert(&base_values, "car", Some(&car_values))
            .expect("the insert is built")
            .build(),
        Vehicle::insert(
            vehicle::ActiveModel {
                name: pgorm::set("mini"),
                ..Default::default()
            },
            VehicleActiveModel::Car(car::ActiveModel {
                seats: pgorm::set(4_i16),
                ..Default::default()
            }),
        )
        .expect("the insert is built")
        .build()
    );

    let keyed = values(
        &base(),
        vec![
            ("id".to_owned(), ActiveValue::Unchanged(Value::from(7))),
            set("name", "cooper"),
        ],
    );
    let truck_values = values(&truck(), vec![set("axles", 3_i16)]);
    assert_eq!(
        shape
            .update(&keyed, "truck", Some(&truck_values))
            .expect("the update is built")
            .build(),
        Vehicle::update(
            vehicle::ActiveModel {
                id: ActiveValue::Unchanged(7),
                name: pgorm::set("cooper"),
                ..Default::default()
            },
            VehicleActiveModel::Truck(truck::ActiveModel {
                axles: pgorm::set(3_i16),
                ..Default::default()
            }),
        )
        .expect("the update is built")
        .build()
    );

    assert_eq!(
        shape
            .change_kind(vec![Value::from(7)], "bike", None)
            .expect("the change is built")
            .build(),
        Vehicle::change_kind(7, VehicleActiveModel::Bike)
            .expect("the change is built")
            .build()
    );
}

fn refusal(outcome: Result<DisjointShape, Error>) -> String {
    match outcome {
        Err(Error::Type(message)) => message,
        other => panic!("expected the shape refused as a type error, got {other:?}"),
    }
}

// [spec:pgorm:req:bindings.disjoint+1/test]    what the types of a `#[derive(Disjoint)]` enum hold is refused as a shape is described: no kinds, a kind twice, one table for two kinds, a base without the kind column, a kind table keyed by other types, and what `check` refuses of the kind column and of a kind's key
#[test]
fn a_described_shape_refuses_what_types_hold() {
    let kinds = |kinds: Vec<(&str, Option<DisjointTable>)>| {
        kinds
            .into_iter()
            .map(|(value, table)| (value.to_owned(), table))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        refusal(DisjointShape::new(base(), "kind", kind_type(), Vec::new())),
        "the disjoint entity over `vehicle` declares no kinds: it has at least one"
    );
    assert_eq!(
        refusal(DisjointShape::new(
            base(),
            "kind",
            kind_type(),
            kinds(vec![("car", Some(car())), ("car", None)])
        )),
        "the disjoint entity over `vehicle` declares the kind `car` twice"
    );
    assert_eq!(
        refusal(DisjointShape::new(
            base(),
            "kind",
            kind_type(),
            kinds(vec![("car", Some(car())), ("truck", Some(car()))])
        )),
        "`car` is the table of the kind `truck` and of the kind `car`: each kind has a table \
         of its own"
    );
    assert_eq!(
        refusal(DisjointShape::new(
            base(),
            "category",
            kind_type(),
            kinds(vec![("car", Some(car()))])
        )),
        "`vehicle` has no column `category`, the kind column a disjoint entity's base holds \
         each row's kind in"
    );
    let wide = table(
        "wide",
        vec![DisjointColumn::new("id", ColumnType::BigInteger.def())],
        &["id"],
    );
    assert_eq!(
        refusal(DisjointShape::new(
            base(),
            "kind",
            kind_type(),
            kinds(vec![("car", Some(wide))])
        )),
        "the key of `wide`, the table of the kind `car`, is (`BigInteger`), not the base \
         `vehicle`'s (`Integer`): a kind's row is keyed by its base row's key"
    );
    assert_eq!(
        refusal(DisjointShape::new(
            base(),
            "kind",
            kind_type(),
            kinds(vec![("car", Some(car())), ("truck", Some(truck()))])
        )),
        "the kind column `vehicle`.`kind` is `vehicle_kind` ('car', 'truck', 'bike'), not the \
         kind type `vehicle_kind` ('car', 'truck')"
    );
    let nullable = table(
        "vehicle",
        vec![
            DisjointColumn::new("id", ColumnType::Integer.def()),
            DisjointColumn::new("kind", vehicle_kind().def().nullable()),
        ],
        &["id"],
    );
    assert_eq!(
        refusal(DisjointShape::new(
            nullable,
            "kind",
            kind_type(),
            kinds(vec![("car", Some(car())), ("truck", None), ("bike", None)])
        )),
        "the kind column `vehicle`.`kind` is nullable: every base row of a disjoint entity has \
         a kind, so the column is `NOT NULL`"
    );
    let drawn = table(
        "vehicle",
        vec![
            DisjointColumn::new("id", ColumnType::Integer.def()),
            DisjointColumn::new("kind", vehicle_kind().def().identity()),
        ],
        &["id"],
    );
    assert_eq!(
        refusal(DisjointShape::new(
            drawn,
            "kind",
            kind_type(),
            kinds(vec![("car", None), ("truck", None), ("bike", None)])
        )),
        "the kind column `vehicle`.`kind` is an identity: a write sets a base row's kind to the \
         kind it writes, so no sequence draws it"
    );
    let defaulted = table(
        "car",
        vec![
            DisjointColumn::new("id", ColumnType::Integer.def()).with_unstated_default(),
            DisjointColumn::new("seats", ColumnType::SmallInteger.def()),
        ],
        &["id"],
    );
    assert_eq!(
        refusal(DisjointShape::new(
            base(),
            "kind",
            kind_type(),
            kinds(vec![
                ("car", Some(defaulted)),
                ("truck", None),
                ("bike", None)
            ])
        )),
        "`car`.`id`, the key of the kind `car`, is given a default: a kind's row takes its key \
         from its base row, so the database does not generate it"
    );
    let identity = table(
        "car",
        vec![DisjointColumn::new(
            "id",
            ColumnType::Integer.def().identity(),
        )],
        &["id"],
    );
    assert_eq!(
        refusal(DisjointShape::new(
            base(),
            "kind",
            kind_type(),
            kinds(vec![
                ("car", Some(identity)),
                ("truck", None),
                ("bike", None)
            ])
        )),
        "`car`.`id`, the key of the kind `car`, is an identity: a kind's row takes its key from \
         its base row, so the database does not generate it"
    );
    match DisjointTable::new(
        TableName::Table(Name::runtime("keyless")),
        vec![DisjointColumn::new("id", ColumnType::Integer.def())],
        &[],
    ) {
        Err(Error::Type(message)) => assert_eq!(
            message,
            "`keyless` has no primary key, which a disjoint entity's tables are keyed by"
        ),
        other => panic!("expected a keyless table refused, got {other:?}"),
    }
}

/// Every row `query` reads, as the judge reads them.
async fn rows(db: &DatabaseConnection, query: &SelectStatement) -> Result<Vec<QueryResult>, Error> {
    let (sql, values) = query.build();
    let holders: Vec<ValueHolder> = values.into_iter().map(ValueHolder).collect();
    let params: Vec<&(dyn ToSql + Sync)> = holders
        .iter()
        .map(|holder| holder as &(dyn ToSql + Sync))
        .collect();
    Ok(db
        .query_all(&sql, &params)
        .await?
        .into_iter()
        .map(QueryResult::from)
        .collect())
}

/// One row as `(id, kind, name, seats or axles)`: the base's columns under
/// `s0_`, and the kind's own column under the source the judge names.
fn read(
    shape: &DisjointShape,
    row: &QueryResult,
) -> Result<(i32, String, String, Option<i16>), Error> {
    let id: i32 = row.try_get("s0_", "id")?;
    let judged = shape.judge(row, || vec![id.to_string()])?;
    let kind = shape.kinds()[judged.kind].value().to_owned();
    let extra = match (judged.source, kind.as_str()) {
        (Some(source), "car") => Some(row.try_get(&format!("s{source}_"), "seats")?),
        (Some(source), "truck") => Some(row.try_get(&format!("s{source}_"), "axles")?),
        _ => None,
    };
    Ok((id, kind, row.try_get("s0_", "name")?, extra))
}

// [spec:pgorm:req:bindings.disjoint+1/test]    a described shape's schema runs on a live server, its writes return the row a read gives and miss as `RecordNotFound` would, and its read, narrowed, paged, counted and read through a keyset cursor, judges each row by its kind
#[pgorm_macros::test]
async fn a_described_shape_runs_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_shape").await;
    let db = ctx.db.get().await?;
    let shape = vehicles();
    for statement in shape.create()? {
        db.execute(&statement.to_string(), &[]).await?;
    }

    let mini = shape.insert(
        &values(&base(), vec![set("name", "mini")]),
        "car",
        Some(&values(&car(), vec![set("seats", 4_i16)])),
    )?;
    let lorry = shape.insert(
        &values(&base(), vec![set("name", "lorry")]),
        "truck",
        Some(&values(
            &truck(),
            vec![set("axles", 3_i16), set("payload_kg", 9000)],
        )),
    )?;
    let bmx = shape.insert(&values(&base(), vec![set("name", "bmx")]), "bike", None)?;
    let mut written = Vec::new();
    for write in [&mini, &lorry, &bmx] {
        let rows = rows(&db, write).await?;
        assert_eq!(rows.len(), 1);
        written.push(read(&shape, &rows[0])?);
    }
    assert_eq!(
        written,
        [
            (1, "car".to_owned(), "mini".to_owned(), Some(4)),
            (2, "truck".to_owned(), "lorry".to_owned(), Some(3)),
            (3, "bike".to_owned(), "bmx".to_owned(), None),
        ]
    );

    let mut ordered = shape.find();
    ordered.order_by(
        (Name::runtime("vehicle"), Name::runtime("id")),
        pgorm::pgorm_query::Order::Asc,
    );
    let read_all = |query: SelectStatement| {
        let db = &db;
        let shape = &shape;
        async move {
            rows(db, &query)
                .await?
                .iter()
                .map(|row| read(shape, row))
                .collect::<Result<Vec<_>, Error>>()
        }
    };
    assert_eq!(read_all(ordered.clone()).await?, written);
    assert_eq!(
        read_all(with(ordered.clone(), shape.kind_is("truck")?)).await?,
        written[1..2]
    );

    let page = page_statement(&ordered, NonZeroU64::new(2).expect("two"), 1)?;
    assert_eq!(read_all(page).await?, written[2..]);
    let counted = rows(&db, &count_statement(&ordered)?).await?;
    assert_eq!(counted[0].try_get::<i64>("", "num_items")?, 3);

    let mut cursor = shape.cursor_by(shape.find(), &["id"])?;
    cursor.after(1).first(1);
    assert_eq!(read_all(cursor.statement()?).await?, written[1..2]);
    let mut last = shape.cursor_by(shape.find(), &["id"])?;
    last.last(2);
    let mut window = read_all(last.statement()?).await?;
    assert!(last.reversed());
    window.reverse();
    assert_eq!(window, written[1..]);

    let renamed = shape.update(
        &values(
            &base(),
            vec![
                ("id".to_owned(), ActiveValue::Unchanged(Value::from(1))),
                set("name", "cooper"),
            ],
        ),
        "car",
        Some(&values(&car(), vec![set("seats", 5_i16)])),
    )?;
    assert_eq!(
        read(&shape, &rows(&db, &renamed).await?[0])?,
        (1, "car".to_owned(), "cooper".to_owned(), Some(5))
    );
    let wrong_kind = shape.update(
        &values(
            &base(),
            vec![("id".to_owned(), ActiveValue::Unchanged(Value::from(1)))],
        ),
        "truck",
        Some(&values(&truck(), vec![set("axles", 1_i16)])),
    )?;
    assert!(
        rows(&db, &wrong_kind).await?.is_empty(),
        "no row of that kind"
    );

    let rekinded = shape.change_kind(
        vec![Value::from(1)],
        "truck",
        Some(&values(
            &truck(),
            vec![set("axles", 2_i16), set("payload_kg", 500)],
        )),
    )?;
    assert_eq!(
        read(&shape, &rows(&db, &rekinded).await?[0])?,
        (1, "truck".to_owned(), "cooper".to_owned(), Some(2))
    );
    assert!(
        rows(
            &db,
            &shape.change_kind(
                vec![Value::from(1)],
                "truck",
                Some(&values(&truck(), vec![]))
            )?
        )
        .await?
        .is_empty(),
        "a row already of the kind is no row"
    );

    match shape.insert(&values(&base(), vec![set("kind", "bike")]), "bike", None) {
        Err(Error::Type(message)) => assert_eq!(
            message,
            "`vehicle`.`kind` holds a value: it is the kind column, which an insert sets to the \
             kind it is given, so the base's `ActiveModel` leaves it `NotSet`"
        ),
        other => panic!("expected the kind column refused, got {other:?}"),
    }
    match shape.insert(&values(&base(), vec![]), "car", None) {
        Err(Error::Type(message)) => assert_eq!(
            message,
            "the kind `car` has a table, `car`, so a write of it gives its row's values"
        ),
        other => panic!("expected the missing row refused, got {other:?}"),
    }
    match shape.update(&values(&base(), vec![]), "bike", None) {
        Err(Error::PrimaryKeyNotSet) => {}
        other => panic!("expected a missing key refused, got {other:?}"),
    }

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:bindings.disjoint+1/test]    the judge reads each row's kind from its kind column and finds its own kind's row by the absence witness, a row missing its kind's row or holding another kind's being the error a typed read reports
#[pgorm_macros::test]
async fn a_described_shape_judges_inconsistent_rows_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_shape_judge").await;
    let db = ctx.db.get().await?;
    let shape = vehicles();
    let row = |sql: &'static str| {
        let db = &db;
        async move { db.query_one(sql, &[]).await.map(QueryResult::from) }
    };
    let judged = |row: &QueryResult| shape.judge(row, || vec!["9".to_owned()]);

    let car = row(
        "SELECT 9 AS s0_id, 'car' AS s0_kind, 'x' AS s0_name, 9 AS s1_id, 4::int2 AS s1_seats, \
         NULL::int AS s2_vehicle_id, NULL::int2 AS s2_axles, NULL::int AS s2_payload_kg",
    )
    .await?;
    assert_eq!(
        judged(&car)?,
        DisjointJudged {
            kind: 0,
            source: Some(1)
        }
    );
    let bike = row(
        "SELECT 9 AS s0_id, 'bike' AS s0_kind, 'x' AS s0_name, NULL::int AS s1_id, \
         NULL::int2 AS s1_seats, NULL::int AS s2_vehicle_id, NULL::int2 AS s2_axles, \
         NULL::int AS s2_payload_kg",
    )
    .await?;
    assert_eq!(
        judged(&bike)?,
        DisjointJudged {
            kind: 2,
            source: None
        }
    );
    let stray = row(
        "SELECT 9 AS s0_id, 'car' AS s0_kind, 'x' AS s0_name, NULL::int AS s1_id, \
         NULL::int2 AS s1_seats, 9 AS s2_vehicle_id, 2::int2 AS s2_axles, 1 AS s2_payload_kg",
    )
    .await?;
    match judged(&stray) {
        Err(Error::Query(error)) => assert_eq!(
            error.to_string(),
            "the `vehicle` row whose key is (`id` = 9) is of the kind `car` but has no row in \
             `car`, its kind's table and a row in `truck`, the table of the kind `truck`: the \
             constraints `Schema::create_disjoint` writes hold each base row to exactly its own \
             kind's table, so this schema was not created by it"
        ),
        other => panic!("expected the row reported as inconsistent, got {other:?}"),
    }
    let unknown = row(
        "SELECT 9 AS s0_id, 'boat' AS s0_kind, 'x' AS s0_name, NULL::int AS s1_id, \
         NULL::int2 AS s1_seats, NULL::int AS s2_vehicle_id, NULL::int2 AS s2_axles, \
         NULL::int AS s2_payload_kg",
    )
    .await?;
    assert!(matches!(judged(&unknown), Err(Error::Type(_))));

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.write+2/test]    a kind's table with no schema, named as its schema-qualified base, is read under its name with `_2` appended, and every write of the shape is refused as `Error::Type` naming both before anything is sent, since its part would be read for the kind's table
#[test]
fn unqualified_kind_named_as_base_is_unwritten() {
    let qualified = DisjointTable::new(
        TableName::SchemaTable(Name::runtime("market"), Name::runtime("vehicle")),
        base().columns().to_vec(),
        &["id"],
    )
    .expect("the table is described");
    let named_as_base = table("vehicle", car().columns().to_vec(), &["id"]);
    let shape = DisjointShape::new(
        qualified.clone(),
        "kind",
        kind_type(),
        vec![
            ("car".to_owned(), Some(named_as_base.clone())),
            ("truck".to_owned(), Some(truck())),
            ("bike".to_owned(), None),
        ],
    )
    .expect("the shape is described");
    let read = shape.find().to_string();
    assert!(
        read.contains(
            r#"FROM "market"."vehicle" LEFT JOIN "vehicle" AS "vehicle_2" ON "vehicle"."id" = "vehicle_2"."id""#
        ),
        "{read}"
    );
    assert!(
        read.contains(r#""vehicle_2"."seats" AS "s1_seats""#),
        "{read}"
    );

    let refusal = "`vehicle` has no schema and is named as `market.vehicle`, whose part of a \
                   write is named `vehicle`, so a write would read that part for it: qualify \
                   `vehicle` by its schema";
    let refused = |outcome: Result<SelectStatement, Error>| match outcome {
        Err(Error::Type(message)) => message,
        other => panic!("expected the write refused, got {other:?}"),
    };
    let base_values = values(&qualified, vec![set("name", "mini")]);
    let seats = values(&named_as_base, vec![set("seats", 4_i16)]);
    assert_eq!(
        refused(shape.insert(&base_values, "car", Some(&seats))),
        refusal
    );
    let keyed = values(
        &qualified,
        vec![("id".to_owned(), ActiveValue::Unchanged(Value::Int(Some(1))))],
    );
    assert_eq!(refused(shape.update(&keyed, "car", Some(&seats))), refusal);
    assert_eq!(
        refused(shape.change_kind(vec![Value::Int(Some(1))], "bike", None)),
        refusal
    );
}
