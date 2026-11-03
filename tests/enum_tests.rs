#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::{
    ActiveEnum, ActiveEnumValue, Error, QueryResult, TryFromU64, Value, entity::prelude::*,
};
use pgorm_query::{ArrayType, Expr, Query, QueryBuilder, SimpleExpr, StringLen};
use pretty_assertions::assert_eq;

/// Render a bare expression by projecting it and stripping the `SELECT `.
fn expr_sql(e: SimpleExpr) -> String {
    Query::select()
        .expr(e)
        .to_string()
        .strip_prefix("SELECT ")
        .expect("select statement")
        .to_owned()
}

// ---------------------------------------------------------------------------
// entity.traits.active-enum
// ---------------------------------------------------------------------------

// [spec:pgorm:def:entity.traits.active-enum+2/test]    the `ActiveEnum` surface:
// `name()` as the database enum's identifier, `to_value` / `into_value` mapping
// a variant to its backing value, `try_from_value` as the fallible reverse
// (`Error` for an unknown value), `db_type()` as the column definition,
// `as_enum()` wrapping a value in a cast to the enum's type name, and `values()`
// enumerating every variant in iterator order
#[test]
fn active_enum_trait_surface() {
    // `name()` is the database enum identifier: the explicit `enum_name` when
    // one was given, and the Rust type name in snake_case otherwise.
    assert_eq!(Tea::name().to_string(), "tea");
    assert_eq!(Category::name().to_string(), "category");
    assert_eq!(Color::name().to_string(), "color");
    assert_eq!(MediaType::name().to_string(), "media_type");

    // `Value` is the backing Rust type: `String` for Tea/Category, `i32` for Color.
    let _: String = Tea::EverydayTea.to_value();
    let _: i32 = Color::Black.to_value();

    // `to_value` borrows, `into_value` consumes; both land on the same value.
    assert_eq!(Tea::EverydayTea.to_value(), "EverydayTea".to_owned());
    assert_eq!(Tea::BreakfastTea.to_value(), "BreakfastTea".to_owned());
    assert_eq!(Tea::EverydayTea.into_value(), "EverydayTea".to_owned());
    assert_eq!(Category::Big.to_value(), "B".to_owned());
    assert_eq!(Category::Small.to_value(), "S".to_owned());
    assert_eq!(Color::Black.to_value(), 0);
    assert_eq!(Color::White.to_value(), 1);

    // `try_from_value` is the reverse mapping; unknown values are an `Error`.
    assert_eq!(
        Tea::try_from_value(&"EverydayTea".to_owned()).unwrap(),
        Tea::EverydayTea
    );
    assert_eq!(
        Category::try_from_value(&"S".to_owned()).unwrap(),
        Category::Small
    );
    assert_eq!(Color::try_from_value(&1).unwrap(), Color::White);

    let err = Tea::try_from_value(&"OolongTea".to_owned()).unwrap_err();
    assert!(
        matches!(err, Error::Type(_)),
        "unknown value must be an Error::Type, got {err:?}"
    );
    assert!(Color::try_from_value(&99).is_err());

    // Round trip over every variant.
    for tea in Tea::iter() {
        assert_eq!(Tea::try_from_value(&tea.to_value()).unwrap(), tea);
    }

    // `db_type()` is the column definition used for the enum column. `Tea` is a
    // real database enum, so its type carries the enum name; `Category` was
    // declared as a plain `String` column, so it does not.
    assert_eq!(
        Tea::db_type().get_enum_name().map(|n| n.to_string()),
        Some("tea".to_owned())
    );
    assert_eq!(
        Category::db_type().get_column_type(),
        &ColumnType::String(StringLen::N(1))
    );
    assert_eq!(Category::db_type().get_enum_name(), None);
    assert_eq!(Color::db_type().get_column_type(), &ColumnType::Integer);

    // `as_enum()` wraps the variant's value in a cast to the enum type name.
    assert_eq!(
        expr_sql(Tea::EverydayTea.as_enum()),
        r#"CAST('EverydayTea' AS tea)"#
    );
    assert_eq!(expr_sql(Color::White.as_enum()), r#"CAST(1 AS color)"#);

    // `values()` enumerates every variant's database value, in iterator order.
    assert_eq!(
        Tea::values(),
        ["EverydayTea".to_owned(), "BreakfastTea".to_owned()]
    );
    assert_eq!(Category::values(), ["B".to_owned(), "S".to_owned()]);
    assert_eq!(Color::values(), [0, 1]);
    assert_eq!(
        Tea::values(),
        Tea::iter().map(|t| t.to_value()).collect::<Vec<_>>(),
        "values() must follow Iterable order"
    );

    // The `ValueVec` associated type exists but carries no meaning; it is only
    // pinned here so its documented removal is a visible break.
    let _: <Tea as ActiveEnum>::ValueVec = Vec::<String>::new();
}

// ---------------------------------------------------------------------------
// entity.traits.active-enum.limits
// ---------------------------------------------------------------------------

fn assert_is_active_enum_value<T: ActiveEnumValue>() {}

// [spec:pgorm:req:entity.traits.active-enum.limits+2/test]    `ActiveEnumValue` is
// implemented for exactly `String`, `i8`, `i16`, `i32`, `i64` and `u32`, and the
// blanket `TryFromU64` impl for every `ActiveEnum` returns `Error::ConvertFromU64`
// — which is why an active-enum primary key has to declare `auto_increment = false`
#[test]
fn active_enum_value_limits() {
    // The six backing types the trait is implemented for.
    assert_is_active_enum_value::<String>();
    assert_is_active_enum_value::<i8>();
    assert_is_active_enum_value::<i16>();
    assert_is_active_enum_value::<i32>();
    assert_is_active_enum_value::<i64>();
    assert_is_active_enum_value::<u32>();

    // Every `ActiveEnum` is `TryFromU64`, and every such conversion refuses.
    let err = <Tea as TryFromU64>::try_from_u64(1).unwrap_err();
    assert!(
        matches!(err, Error::ConvertFromU64(_)),
        "expected ConvertFromU64, got {err:?}"
    );
    assert!(<Category as TryFromU64>::try_from_u64(0).is_err());
    assert!(<Color as TryFromU64>::try_from_u64(u64::MAX).is_err());

    // Which is exactly why `teas`, whose primary key is a `Tea`, must declare
    // `auto_increment = false`: an auto-increment key would need to build itself
    // from the returned u64.
    assert!(!<teas::PrimaryKey as PrimaryKeyTrait>::auto_increment());
}

/// Probes the `u32` branch of `try_get_vec_by`, which cannot be constructed
/// without a real `QueryResult`.
#[derive(Debug)]
struct U32VecProbe;

impl FromQueryResult for U32VecProbe {
    fn from_query_result(res: &QueryResult, _pre: &str) -> Result<Self, Error> {
        <u32 as ActiveEnumValue>::try_get_vec_by(res, "ids")?;
        Ok(Self)
    }
}

// [spec:pgorm:req:entity.traits.active-enum.limits+2/test]    reading an array of
// enum values is a Postgres-only capability, and `u32` is not covered by
// `postgres-array`, so `try_get_vec_by` errors rather than decoding
#[pgorm_macros::test]
fn active_enum_u32_vec_read_errors() {
    let ctx = TestContext::new("active_enum_u32_vec_read_errors").await;
    let db = ctx.db.get().await.unwrap();

    let err = U32VecProbe::find_by_statement(r#"SELECT ARRAY[1, 2] AS "ids""#, vec![])
        .one(&db)
        .await
        .expect_err("a `u32` enum array cannot be decoded");
    assert!(
        matches!(err, Error::Type(ref msg) if msg.contains("postgres-array")),
        "expected a postgres-array type error, got {err:?}"
    );

    drop(db);
    ctx.delete().await;
}

// ---------------------------------------------------------------------------
// entity.traits.column.enum-cast
// ---------------------------------------------------------------------------

/// A column set spanning the cases the cast rule distinguishes: a real database
/// enum, an array of one, a plain non-enum column, and a JSON column.
mod casts {
    use super::pgorm_active_enums::Tea;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "casts")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub tea: Tea,
        pub teas: Vec<Tea>,
        pub name: String,
        pub payload: Json,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

// [spec:pgorm:sem:entity.traits.column.enum-cast+15/test]    on read, `select_as` /
// `select_enum_as` casts an enum column to `text` — `text[]` when the column is
// an `Array` of an enum — and leaves non-enum columns alone; on write,
// `save_as` / `save_enum_as` casts the value to the enum's database type name,
// or `{enum_name}[]` for arrays
#[test]
fn enum_columns_are_cast_at_the_sql_boundary() {
    // Read side: the projection casts the enum column to text, the array of
    // enums to text[], and leaves the plain column untouched.
    assert_eq!(
        casts::Entity::find().build().0,
        [
            r#"SELECT "casts"."id","#,
            r#"CAST("casts"."tea" AS text),"#,
            r#"CAST("casts"."teas" AS text[]),"#,
            r#""casts"."name","#,
            r#""casts"."payload""#,
            r#"FROM "casts""#,
        ]
        .join(" ")
    );

    // The same through `select_enum_as` called directly.
    assert_eq!(
        expr_sql(casts::Column::Tea.select_enum_as(Expr::col(casts::Column::Tea))),
        r#"CAST("tea" AS text)"#
    );
    assert_eq!(
        expr_sql(casts::Column::Teas.select_enum_as(Expr::col(casts::Column::Teas))),
        r#"CAST("teas" AS text[])"#
    );
    // A non-enum column passes through unchanged: no CAST at all.
    assert_eq!(
        expr_sql(casts::Column::Name.select_enum_as(Expr::col(casts::Column::Name))),
        r#""name""#
    );
    assert_eq!(
        expr_sql(casts::Column::Id.select_as(Expr::col(casts::Column::Id))),
        r#""id""#
    );

    // Write side: the value is cast to the enum's own database type name.
    assert_eq!(
        expr_sql(casts::Column::Tea.save_enum_as(Expr::val("EverydayTea"))),
        r#"CAST('EverydayTea' AS tea)"#
    );
    // ...and to `{enum_name}[]` for an array column.
    assert_eq!(
        expr_sql(casts::Column::Teas.save_enum_as(Expr::val("EverydayTea"))),
        r#"CAST('EverydayTea' AS tea[])"#
    );
    // A non-enum column is not cast on write either.
    assert_eq!(
        expr_sql(casts::Column::Name.save_as(Expr::val("plain"))),
        r#"'plain'"#
    );

    // Because the comparison operators route their operands through `save_as`,
    // filtering on an enum column compares against a properly cast value while
    // filtering on a plain column does not.
    let filter_sql = |e: SimpleExpr| {
        casts::Entity::find()
            .filter(e)
            .as_query()
            .to_string()
            .split_once(" WHERE ")
            .expect("a WHERE clause")
            .1
            .to_owned()
    };
    assert_eq!(
        filter_sql(casts::Column::Tea.eq(Tea::EverydayTea)),
        r#""casts"."tea" = CAST('EverydayTea' AS tea)"#
    );
    assert_eq!(
        filter_sql(casts::Column::Name.eq("plain")),
        r#""casts"."name" = 'plain'"#
    );
    // Set membership goes through `save_as` per element too.
    assert_eq!(
        filter_sql(casts::Column::Tea.is_in([Tea::EverydayTea, Tea::BreakfastTea])),
        r#""casts"."tea" IN (CAST('EverydayTea' AS tea), CAST('BreakfastTea' AS tea))"#
    );

    // The column-to-column siblings take the other road: the operand is already
    // an enum column on the server, so no cast is applied to either side. This
    // is why `eq` was not widened to admit a column — a widened bound would have
    // dropped the cast above without saying so.
    // [spec:pgorm:def:entity.traits.column+7/test]    the `_col` family does not
    // route its operand through `save_as`
    assert_eq!(
        filter_sql(casts::Column::Tea.eq_col(casts::Column::Tea)),
        r#""casts"."tea" = "casts"."tea""#
    );

    // An INSERT written through the entity carries the same casts.
    assert_eq!(
        Insert::one(casts::ActiveModel {
            id: NotSet,
            tea: set(Tea::BreakfastTea),
            teas: set(vec![Tea::EverydayTea]),
            name: set("plain"),
            payload: set(serde_json::json!({"k": "v"})),
        })
        .as_query()
        .to_string(),
        [
            r#"INSERT INTO "casts" ("tea", "teas", "name", "payload")"#,
            r#"VALUES (CAST('BreakfastTea' AS tea), CAST(ARRAY ['EverydayTea'] AS tea[]), 'plain', '{"k":"v"}')"#,
        ]
        .join(" ")
    );
}

/// A range type a schema created and the multirange created beside it, the
/// two column types whose casts the created-range rule adds.
mod bookings {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
    #[pgorm(range_name = "slot", schema_name = "booking")]
    pub struct Slot(pub Range<i32>);

    #[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
    #[pgorm(multirange_name = "slot_set", schema_name = "booking")]
    pub struct Slots(pub Multirange<i32>);

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "bookings")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub slot: Slot,
        pub slots: Slots,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

// [spec:pgorm:req:sql.ast.cast-shape+2/test]    the entity layer's casts — the
// enum defaults on read and write, `ActiveEnum::as_enum`, a derived
// `select_as` / `save_as` override, and the created-range casts:
// `CreatedRange::into_expr`, the write cast of a created range or multirange
// column and the read cast of a created multirange one — are each the one
// `AsEnum` node, with the quoted-or-verbatim choice carried in its `TypeName`
#[test]
fn entity_layer_casts_build_the_one_cast_node() {
    let tea = || SimpleExpr::from(Expr::col(casts::Column::Tea));
    let teas = || SimpleExpr::from(Expr::col(casts::Column::Teas));
    let name = || SimpleExpr::from(Expr::col(collection::Column::Name));
    let value = || SimpleExpr::from(Expr::val("EverydayTea"));
    let array = || SimpleExpr::from(Expr::val(Value::array(["EverydayTea".to_owned()])));
    let slot = || SimpleExpr::from(Expr::col(bookings::Column::Slot));
    let slots = || SimpleExpr::from(Expr::col(bookings::Column::Slots));
    let text = |text: &'static str| SimpleExpr::from(Expr::val(text));

    // (spelling, the expression it built, the operand it was handed, the
    // type's text, whether that text is the caller's verbatim SQL)
    let spellings = [
        (
            "select_enum_as",
            casts::Column::Tea.select_enum_as(Expr::expr(tea())),
            tea(),
            "text",
            false,
        ),
        (
            "select_enum_as over an enum array",
            casts::Column::Teas.select_enum_as(Expr::expr(teas())),
            teas(),
            "text[]",
            false,
        ),
        (
            "save_enum_as",
            casts::Column::Tea.save_enum_as(Expr::expr(value())),
            value(),
            "tea",
            false,
        ),
        (
            "save_enum_array_as",
            casts::Column::Tea.save_enum_array_as(Expr::expr(array())),
            array(),
            "tea[]",
            false,
        ),
        (
            "ActiveEnum::as_enum",
            Tea::EverydayTea.as_enum(),
            value(),
            "tea",
            false,
        ),
        (
            "derived select_as",
            collection::Column::Name.select_as(Expr::expr(name())),
            name(),
            "text",
            true,
        ),
        (
            "derived save_as",
            collection::Column::Name.save_as(Expr::expr(value())),
            value(),
            "citext",
            true,
        ),
        (
            "derived save_array_as",
            collection::Column::Name.save_array_as(Expr::expr(array())),
            array(),
            "citext[]",
            true,
        ),
        (
            "CreatedRange::into_expr",
            bookings::Slot(Range::from(1..5)).into_expr(),
            text("[1,5)"),
            "booking.slot",
            false,
        ),
        (
            "save_enum_as on a created range, a built-in range value written as its text",
            bookings::Column::Slot.save_enum_as(Expr::val(Range::from(1..5))),
            text("[1,5)"),
            "booking.slot",
            false,
        ),
        (
            "save_enum_as on a created multirange",
            bookings::Column::Slots.save_enum_as(Expr::val(Multirange::from(vec![
                Range::from(1..5),
                Range::from(7..9),
            ]))),
            text("{[1,5),[7,9)}"),
            "booking.slot_set",
            false,
        ),
        (
            "select_enum_as on a created multirange",
            bookings::Column::Slots.select_enum_as(Expr::expr(slots())),
            slots(),
            "text",
            false,
        ),
    ];

    for (spelling, cast, operand, type_text, verbatim) in spellings {
        match cast {
            SimpleExpr::AsEnum(type_name, cast_operand) => {
                assert_eq!(
                    *cast_operand, operand,
                    "{spelling}: the operand rides untouched"
                );
                assert_eq!(
                    (type_name.raw_text().as_str(), type_name.is_verbatim()),
                    (type_text, verbatim),
                    "{spelling}: the type, and how it renders, live in the `TypeName`"
                );
            }
            other => panic!("{spelling} built a cast that is not an `AsEnum`: {other:?}"),
        }
    }

    // A created range is read from its binary form, so its read is no cast.
    assert_eq!(
        bookings::Column::Slot.select_enum_as(Expr::expr(slot())),
        slot()
    );
}

// [spec:pgorm:sem:entity.traits.column.enum-cast+15/test]    the special case:
// under `with-json` + `postgres-array`, saving into a `Json` / `JsonBinary`
// column flattens a `Value::Array` of JSON values into a single `Value::Json`
// array value instead of applying an enum cast
#[test]
fn json_column_flattens_json_array_without_cast() {
    use serde_json::json;

    // A `Value::Array(ArrayType::Json, ..)` bound for a Json column collapses to
    // one Json value holding the array, rather than staying an array binding.
    let flattened = casts::Column::Payload.save_as(Expr::val(Value::Array(
        ArrayType::Json,
        Some(Box::new(vec![
            Value::Json(Some(Box::new(json!({"a": 1})))),
            Value::Json(Some(Box::new(json!({"b": 2})))),
        ])),
    )));
    assert_eq!(expr_sql(flattened), r#"'[{"a":1},{"b":2}]'"#);

    // A null array becomes a null Json rather than a null array.
    let flattened = casts::Column::Payload.save_as(Expr::val(Value::Array(ArrayType::Json, None)));
    assert_eq!(expr_sql(flattened), "NULL");

    // Anything else bound for the Json column passes straight through.
    let untouched = casts::Column::Payload.save_as(Expr::val(json!({"k": "v"})));
    assert_eq!(expr_sql(untouched), r#"'{"k":"v"}'"#);
}

// [spec:pgorm:sem:entity.traits.column.enum-cast+15/test]    the casts survive a
// real round trip: values written through the enum cast read back as the
// original variants
#[pgorm_macros::test]
async fn enum_cast_round_trip() -> Result<(), Error> {
    use pgorm::Schema;

    let ctx = TestContext::new("enum_cast_round_trip").await;
    let db = ctx.db.get().await?;

    let schema = Schema::new();
    for stmt in schema.create_enum_from_entity(casts::Entity) {
        db.execute(&stmt.to_string(), &[]).await?;
    }
    db.execute(
        &schema.create_table_from_entity(casts::Entity)?.to_string(),
        &[],
    )
    .await?;

    let inserted = casts::ActiveModel {
        id: NotSet,
        tea: set(Tea::BreakfastTea),
        teas: set(vec![Tea::EverydayTea, Tea::BreakfastTea]),
        name: set("plain"),
        payload: set(serde_json::json!({"k": "v"})),
    }
    .insert(&db)
    .await?;

    assert_eq!(inserted.tea, Tea::BreakfastTea);
    assert_eq!(inserted.teas, [Tea::EverydayTea, Tea::BreakfastTea]);

    // Filtering on the enum column round trips through the write-side cast.
    let found = casts::Entity::find()
        .filter(casts::Column::Tea.eq(Tea::BreakfastTea))
        .one(&db)
        .await?;
    assert_eq!(found, inserted);

    // ...and a variant that was not written matches nothing.
    assert_eq!(
        casts::Entity::find()
            .filter(casts::Column::Tea.eq(Tea::EverydayTea))
            .one_opt(&db)
            .await?,
        None
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
