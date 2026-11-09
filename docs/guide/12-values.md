# Values and types

Every column holds a value of some PostgreSQL type, and every field of a
`Model` is the Rust type that stands for it. This chapter goes through the
types pgorm carries: what each one is in Rust, what it is in PostgreSQL, and
anything that travels between the two in a way you should know about.

## The types at a glance

A field's Rust type decides its column's type, as [Entities](crate::guide::entities)
showed. The full vocabulary is:

| Rust | PostgreSQL |
| --- | --- |
| `bool` | `boolean` |
| `i16`, `i32`, `i64` | `smallint`, `integer`, `bigint` |
| `f32`, `f64` | `real`, `double precision` |
| `Decimal` | `numeric` |
| `String` | `varchar`, or `text` with `column_type = ColumnType::Text` |
| `Vec<u8>` | `bytea` |
| `Date`, `Time`, `DateTime` | `date`, `time`, `timestamp` |
| `DateTimeWithTimeZone` | `timestamptz` |
| `Interval` | `interval` |
| `Uuid` | `uuid` |
| `Json` | `json`, or `jsonb` with `column_type = ColumnType::JsonBinary` |
| a `JsonValueType` | `json` or `jsonb`, holding the type serialized |
| `Vec<T>` | an array of `T`'s type |
| an `ActiveEnum` | the enum's type, or its `db_type` |
| `Range<T>`, `Multirange<T>` | a built-in range or multirange, such as `daterange` |
| a `CreatedRange` | a range type the schema creates |
| `IpNetwork`, `MacAddress` | `inet`, `macaddr` |
| `Vector` | pgvector's `vector` |
| a `ValueType` newtype | the type it wraps |
| `Option<T>` | `T`'s type, nullable, for every `T` above |

Every name in the left column is in the prelude. The date and time types are
[jiff]'s, `Decimal` is [rust_decimal]'s, and `Json` is `serde_json::Value`.
A `money` column and a column typed by a domain take their field's type from
what they hold, and are covered below.

## The `Value` container

When pgorm builds a statement it carries each parameter as a
[`Value`](crate::Value), one enum with a variant per kind of value. Every
variant holds an `Option`, so a typed `NULL` remains a `NULL` of its own type.
Anything that converts into `Value` can be bound, and
[`ValueType`](crate::pgorm_query::ValueType)'s
[`from_value`](crate::pgorm_query::ValueType::from_value) reads one back
out:

```rust
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::ValueType;

let price: Value = Decimal::new(450, 2).into();
assert_eq!(price, Value::Decimal(Some(Box::new(Decimal::new(450, 2)))));

// `None` is the NULL of its own type.
let missing: Value = Option::<String>::None.into();
assert_eq!(missing, Value::String(None));

// Reading back checks the variant.
assert_eq!(Decimal::from_value(price.clone())?, Decimal::new(450, 2));
assert!(i32::from_value(price).is_err());
# Ok::<(), pgorm::pgorm_query::ValueTypeError>(())
```

A statement's `build()` returns its values beside its SQL, as the [query
builder](crate::guide::query_builder) chapter shows, and an `ActiveModel` sets each
column to one. PostgreSQL has no unsigned integers, so there is no `Value` for
`u8` or `u16`, and they cannot be bound at all.

## Dates and times

pgorm uses [jiff] for dates and times. Each PostgreSQL type has the jiff type
that holds exactly what it does:

- `Date` (`jiff::civil::Date`) is a calendar date, `date`.
- `Time` (`jiff::civil::Time`) is a time of day, `time`.
- `DateTime` (`jiff::civil::DateTime`) is a date and time with no time zone,
  `timestamp`.
- `DateTimeWithTimeZone` (`jiff::Timestamp`) is an instant, `timestamptz`.
  PostgreSQL stores the instant and no zone, and so does a `Timestamp`.

The bakery's bakers work shifts. A shift falls on a date, starts at a time of
day, and records the instant the baker clocked in:

```rust
mod shift {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "shift")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub baker_id: i32,
        pub on: Date,
        pub starts: Time,
        pub planned: DateTime,
        pub clocked_in: Option<DateTimeWithTimeZone>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("values_temporal").await;
# let db = scratch.pool.get().await?;
use jiff::civil::{date, time};
use pgorm::entity::prelude::*;
# db.batch_execute(&pgorm::Schema::new().create_table_from_entity(shift::Entity)?.to_string()).await?;

let early = shift::ActiveModel {
    baker_id: set(1),
    on: set(date(2026, 11, 6)),
    starts: set(time(5, 30, 0, 0)),
    planned: set(date(2026, 11, 6).at(5, 30, 0, 0)),
    clocked_in: set(Some("2026-11-06T04:28:00Z".parse::<DateTimeWithTimeZone>()?)),
    ..Default::default()
}
.insert(&db)
.await?;

assert_eq!(early.on, date(2026, 11, 6));
assert_eq!(early.starts.to_string(), "05:30:00");
assert_eq!(early.clocked_in.map(|at| at.to_string()), Some("2026-11-06T04:28:00Z".to_owned()));

// Dates compare as dates.
let this_week = shift::Entity::find()
    .filter(shift::Column::On.between(date(2026, 11, 2), date(2026, 11, 8)))
    .count(&db)
    .await?;
assert_eq!(this_week, 1);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A `timestamptz` comes back as the instant it was, whatever the session's time
zone. To show it in a zone, convert it in Rust with jiff
(`at.in_tz("Europe/Oslo")`), where the zone rules are explicit.

## Numbers

Integers and floats map one to one. `numeric` maps to `Decimal`, which is
exact: `4.50` is stored as `4.50` and read back as `4.50`, with its scale.
Arithmetic the database does on a `numeric` stays exact, so a sum of prices
is the sum to the cent:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("values_decimal").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm_guide_support::order_line;

let line_totals: Vec<Decimal> = order_line::Entity::find()
    .select(Expr::col(order_line::Column::Price).mul(Expr::col(order_line::Column::Quantity)))
    .filter(order_line::Column::OrderId.eq(1))
    .order_by_asc(order_line::Column::Id)
    .into_tuple()
    .all(&db)
    .await?;

assert_eq!(line_totals, [Decimal::new(900, 2), Decimal::new(320, 2)]);
let total: Decimal = line_totals.iter().sum();
assert_eq!(total.to_string(), "12.20");
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A `Decimal` holds up to 28 digits after the point, in a 96-bit coefficient.
A `numeric` beyond that is not read at all rather than read rounded: the row
fails to decode, wherever the `Decimal` is read, a model's field, a tuple, an
array or a range's bound. Give a `numeric` column a precision and scale within
it, such as `decimal(10, 2)` for prices, and its every value reads:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("values_decimal_exact").await;
# let db = scratch.pool.get().await?;
use pgorm::{Values, entity::prelude::*};

let tiny = ("SELECT 0.0000000000000000000000000001::numeric", Values(vec![]))
    .into_tuple::<Decimal>()
    .one(&db)
    .await?;
assert_eq!(tiny, Decimal::new(1, 28));

// One more digit after the point is more than a `Decimal` holds.
let tinier = ("SELECT 0.00000000000000000000000000001::numeric", Values(vec![]))
    .into_tuple::<Decimal>()
    .one(&db)
    .await;
assert!(tinier.is_err());
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Money

PostgreSQL's `money` stores an amount in the currency of the server's
`lc_monetary` setting. pgorm reads and writes it as `numeric`, through a
`Decimal` field: declare the column `column_type = ColumnType::Money`, and the
column is read cast to `numeric` and each value written cast to `money`. The
server rounds a value to the currency's places.

```rust
mod tip_jar {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "tip_jar")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(column_type = ColumnType::Money)]
        pub amount: Decimal,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("values_money").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
# db.batch_execute(&pgorm::Schema::new().create_table_from_entity(tip_jar::Entity)?.to_string()).await?;

let tips = tip_jar::ActiveModel {
    amount: set(Decimal::new(12345, 3)),
    ..Default::default()
}
.insert(&db)
.await?;
// Rounded to the currency's two places.
assert_eq!(tips.amount, Decimal::new(1235, 2));

let (sql, _) = tip_jar::Entity::find()
    .filter(tip_jar::Column::Amount.gt(Decimal::TEN))
    .build();
assert_eq!(
    sql,
    r#"SELECT "tip_jar"."id", CAST("tip_jar"."amount" AS numeric) FROM "tip_jar" WHERE "tip_jar"."amount" > CAST($1::numeric AS money)"#
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## JSON

A `Json` field holds any JSON value, as a `serde_json::Value`. A `json`
column keeps the text as written; a `jsonb` column, declared with
`column_type = ColumnType::JsonBinary`, stores it decomposed, which is what
PostgreSQL indexes and searches.

When the JSON has a fixed shape, give it a type of its own. A struct that
derives `Serialize`, `Deserialize` and
[`JsonValueType`](derive@crate::JsonValueType) is stored as its JSON and read
back as the struct:

```rust
mod profile {
    use pgorm::entity::prelude::*;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonValueType)]
    pub struct Contact {
        pub phone: String,
        pub languages: Vec<String>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "profile")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub baker_id: i32,
        #[pgorm(column_type = ColumnType::JsonBinary)]
        pub contact: Contact,
        pub extra: Json,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("values_json").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use profile::Contact;
use serde_json::json;
# db.batch_execute(&pgorm::Schema::new().create_table_from_entity(profile::Entity)?.to_string()).await?;

let ada = profile::Model {
    baker_id: 1,
    contact: Contact {
        phone: "+47 400 00 001".to_owned(),
        languages: vec!["nb".to_owned(), "en".to_owned()],
    },
    extra: json!({ "apron": "L" }),
};
ada.clone().into_active_model().insert(&db).await?;

// `@>` asks whether the jsonb holds the given JSON.
let english: Vec<profile::Model> = profile::Entity::find()
    .filter(Expr::col(profile::Column::Contact).contains(Expr::val(json!({ "languages": ["en"] }))))
    .all(&db)
    .await?;
assert_eq!(english, [ada]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Arrays

A `Vec<T>` field is an array column of `T`'s type, for each scalar type above
but the network types and `Vector`. `Vec<u8>` is not an array: it is
`bytea`. An array compares as a whole with `eq`, and the
array operators ask about its elements: [`includes`](crate::ColumnTrait::includes)
is `@>` (holds every element of the value), [`contained_by`](crate::ColumnTrait::contained_by)
is `<@`, and [`overlaps`](crate::ColumnTrait::overlaps) is `&&` (shares an
element). An array of an enum is an array of the enum's type.

The other direction, a scalar column against a list of values, is
[`eq_any`](crate::ColumnTrait::eq_any), which binds the whole list as one
array parameter, so a statement has the same SQL however long the list is.
Its items are what `eq` takes, string slices among them:

```rust
mod menu {
    use pgorm::entity::prelude::*;
    use pgorm_guide_support::cake::Flavor;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "menu")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
        pub tags: Vec<String>,
        pub flavors: Vec<Flavor>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("values_arrays").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm_guide_support::cake::Flavor;
# let schema = pgorm::Schema::new();
# for create in schema.create_enum_from_entity(menu::Entity) {
#     db.batch_execute(&create.to_string()).await?;
# }
# db.batch_execute(&schema.create_table_from_entity(menu::Entity)?.to_string()).await?;

let tags = |list: &[&str]| list.iter().map(|tag| tag.to_string()).collect::<Vec<_>>();
Insert::many([
    menu::ActiveModel {
        name: set("Breakfast"),
        tags: set(tags(&["morning", "coffee"])),
        flavors: set(vec![Flavor::Lemon, Flavor::Vanilla]),
        ..Default::default()
    },
    menu::ActiveModel {
        name: set("Party"),
        tags: set(tags(&["evening"])),
        flavors: set(vec![Flavor::Chocolate]),
        ..Default::default()
    },
])
.exec(&db)
.await?;

let with_coffee: Vec<String> = menu::Entity::find()
    .select(menu::Column::Name)
    .filter(menu::Column::Tags.includes(tags(&["coffee"])))
    .into_tuple()
    .all(&db)
    .await?;
assert_eq!(with_coffee, ["Breakfast"]);

let chocolate: Vec<String> = menu::Entity::find()
    .select(menu::Column::Name)
    .filter(menu::Column::Flavors.overlaps(vec![Flavor::Chocolate, Flavor::Carrot]))
    .into_tuple()
    .all(&db)
    .await?;
assert_eq!(chocolate, ["Party"]);

let named: Vec<String> = menu::Entity::find()
    .select(menu::Column::Name)
    .filter(menu::Column::Name.eq_any(["Party", "Picnic"]))
    .into_tuple()
    .all(&db)
    .await?;
assert_eq!(named, ["Party"]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

PostgreSQL arrays carry no dimension in their type, but pgorm reads one
dimension: a `Vec<T>` is a list.

## Enums

An enum deriving [`ActiveEnum`](derive@crate::ActiveEnum) is a PostgreSQL enum
([Entities](crate::guide::entities) declares one). pgorm reads an enum column as its
text, cast to `text`, and writes each value cast to the enum's type, so a
row decodes without knowing the type's internal id and a comparison resolves:

```rust
use pgorm::entity::prelude::*;
use pgorm_guide_support::cake::{self, Flavor};

let (sql, values) = cake::Entity::find()
    .select(cake::Column::Name)
    .filter(cake::Column::Flavor.eq(Flavor::Lemon))
    .build();
assert_eq!(
    sql,
    r#"SELECT "cake"."name" FROM "cake" WHERE "cake"."flavor" = CAST($1::text AS flavor)"#
);
assert_eq!(values.0, ["lemon".into()]);
```

An enum in a schema of its own says `schema_name = ".."`, and its type is
written qualified everywhere it is named. An enum that is not a PostgreSQL
enum, stored as integers or as text, names `rs_type` and `db_type` and needs
no cast.

## UUIDs

A `Uuid` field is a `uuid` column. A key the database generates is written
with `default_expr = Func::uuidv7()`, PostgreSQL 18's time-ordered UUIDs,
which index well because each is greater than the last:

```rust
mod voucher {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "voucher")]
    pub struct Model {
        #[pgorm(primary_key, default_expr = Func::uuidv7())]
        pub code: Uuid,
        pub value: Decimal,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("values_uuid").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
# db.batch_execute(&pgorm::Schema::new().create_table_from_entity(voucher::Entity)?.to_string()).await?;

let first: Uuid = Insert::one(voucher::ActiveModel {
    value: set(Decimal::TEN),
    ..Default::default()
})
.exec_returning_pk(&db)
.await?;
let second: Uuid = Insert::one(voucher::ActiveModel {
    value: set(Decimal::TEN),
    ..Default::default()
})
.exec_returning_pk(&db)
.await?;

assert_eq!(first.get_version_num(), 7);
assert!(first < second);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Intervals

An [`Interval`](crate::pgorm_query::Interval) is PostgreSQL's `interval`: a
number of months, of days and of microseconds, kept apart because a month has
no fixed number of days and a day no fixed number of microseconds across a
daylight-saving change. `Interval::new(months, days, microseconds)` builds
one, and pgorm never turns one field into another, so `1 mon` and `30 days`
are two different values:

```rust
mod recipe {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "recipe")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
        pub proving: Interval,
        pub keeps_for: Interval,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("values_interval").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
# db.batch_execute(&pgorm::Schema::new().create_table_from_entity(recipe::Entity)?.to_string()).await?;

const HOUR: i64 = 3_600_000_000;
let sourdough = recipe::ActiveModel {
    name: set("Sourdough"),
    proving: set(Interval::new(0, 0, 14 * HOUR)),
    keeps_for: set(Interval::new(0, 3, 0)),
    ..Default::default()
}
.insert(&db)
.await?;

assert_eq!(sourdough.proving, Interval::new(0, 0, 14 * HOUR));
assert_eq!(sourdough.keeps_for.to_string(), "+3 days");

let slow: u64 = recipe::Entity::find()
    .filter(recipe::Column::Proving.gt(Interval::new(0, 0, 12 * HOUR)))
    .count(&db)
    .await?;
assert_eq!(slow, 1);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

An interval column can be restricted to some fields, such as `interval
HOUR TO MINUTE`, with
`column_type = ColumnType::Interval(IntervalSpec::Fields(PgInterval::HourToMinute))`.

## Ranges and multiranges

A range is a span of values of one type, such as the dates a seasonal cake is
on sale. PostgreSQL has six built-in range types, and pgorm carries each as a
[`Range<T>`](crate::pgorm_query::Range):

| Rust | PostgreSQL |
| --- | --- |
| `Range<i32>`, `Range<i64>` | `int4range`, `int8range` |
| `Range<Decimal>` | `numrange` |
| `Range<Date>` | `daterange` |
| `Range<DateTime>`, `Range<DateTimeWithTimeZone>` | `tsrange`, `tstzrange` |

A range is either empty or has two bounds, each included, excluded or absent.
The standard library's ranges convert into it: `a..b` is `[a,b)`, `a..=b` is
`[a,b]`, `a..` has no upper bound. A [`Multirange<T>`](crate::pgorm_query::Multirange)
is a set of ranges, `datemultirange` for dates. The column operators of
[Arrays](#arrays) work on ranges too (`includes`, `contained_by`,
`overlaps`), beside [`strictly_left_of`](crate::ColumnTrait::strictly_left_of),
[`adjacent_to`](crate::ColumnTrait::adjacent_to) and the rest:

```rust
mod season {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "season")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub cake: String,
        pub on_sale: Range<Date>,
        pub batch_sizes: Range<i32>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("values_ranges").await;
# let db = scratch.pool.get().await?;
use jiff::civil::date;
use pgorm::entity::prelude::*;
use std::ops::Bound::{Excluded, Included};
# db.batch_execute(&pgorm::Schema::new().create_table_from_entity(season::Entity)?.to_string()).await?;

let gingerbread = season::ActiveModel {
    cake: set("Gingerbread"),
    on_sale: set(Range::from(date(2026, 11, 15)..date(2027, 1, 7))),
    batch_sizes: set(Range::from(12..=48)),
    ..Default::default()
}
.insert(&db)
.await?;

// A discrete range comes back in PostgreSQL's canonical form: [12,49).
assert_eq!(gingerbread.batch_sizes, Range::new(Included(12), Excluded(49)));

// What is on sale over the Christmas week?
let christmas = Range::from(date(2026, 12, 21)..date(2026, 12, 28));
let on_sale: Vec<String> = season::Entity::find()
    .select(season::Column::Cake)
    .filter(season::Column::OnSale.overlaps(christmas))
    .into_tuple()
    .all(&db)
    .await?;
assert_eq!(on_sale, ["Gingerbread"]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

The empty range is a value of its own, `Range::Empty`, and never a pair of
bounds: `[5,5)` reads back as empty. A multirange comes back sorted, with
overlapping and adjacent ranges merged.

## Range types the schema creates

A schema can create a range type of its own, over a type no built-in range
covers, such as `double precision`. Name it once, in a newtype over `Range<T>`
that derives [`CreatedRange`](derive@crate::CreatedRange), and use the
newtype as the field. The type itself is created with the
[query builder](crate::guide::query_builder)'s `Type::create(..).as_range(..)`.

The bakery's ovens each hold a span of temperatures:

```rust
mod oven {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, CreatedRange)]
    #[pgorm(range_name = "temperature")]
    pub struct Temperature(pub Range<f64>);

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "oven")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
        pub heat: Temperature,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("values_created_range").await;
# let db = scratch.pool.get().await?;
use oven::Temperature;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{ColumnType, extension::{RangeDefinition, Type}};

let create_type = Type::create(Name::runtime("temperature"))
    .as_range(RangeDefinition::new(ColumnType::Double))
    .to_string();
assert_eq!(create_type, r#"CREATE TYPE "temperature" AS RANGE (SUBTYPE = double precision)"#);
db.batch_execute(&create_type).await?;
# db.batch_execute(&pgorm::Schema::new().create_table_from_entity(oven::Entity)?.to_string()).await?;

let deck = oven::ActiveModel {
    name: set("Deck oven"),
    heat: set(Temperature(Range::from(180.0..250.5))),
    ..Default::default()
}
.insert(&db)
.await?;
assert_eq!(deck.heat, Temperature(Range::from(180.0..250.5)));

let hot_enough: u64 = oven::Entity::find()
    .filter(oven::Column::Heat.overlaps(Temperature(Range::from(240.0..260.0))))
    .count(&db)
    .await?;
assert_eq!(hot_enough, 1);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A value of a created range type travels as its text, cast to the type by
name, and is read back from its binary form. A range type pgorm creates is
continuous whatever its subtype, since its canonical function would have to
be written in C. The multirange PostgreSQL creates beside it is a newtype over
`Multirange<T>` with `multirange_name = ".."`.

## Domains

A domain is a type with constraints: `CREATE DOMAIN grams AS integer CHECK
(VALUE > 0)`. Its values travel as the type it is over, so the field is that
type, and the column says it is the domain with
`column_type = ColumnType::Domain { name, schema, base }`. The domain's checks
then hold every write:

```rust
mod ingredient {
    use pgorm::entity::prelude::*;

    fn grams() -> ColumnType {
        ColumnType::Domain {
            name: Name::runtime("grams"),
            schema: None,
            base: Arc::new(ColumnType::Integer),
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "ingredient")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
        #[pgorm(column_type = grams())]
        pub per_batch: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("values_domain").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{ColumnType, Domain};
use pgorm::{Schema, SqlError};

let grams = Domain::create(Name::runtime("grams"), ColumnType::Integer)
    .check(Expr::domain_value().gt(0))
    .to_string();
assert_eq!(grams, r#"CREATE DOMAIN "grams" AS integer CHECK (VALUE > 0)"#);
db.batch_execute(&grams).await?;

let create = Schema::new().create_table_from_entity(ingredient::Entity)?.to_string();
assert_eq!(
    create,
    r#"CREATE TABLE "ingredient" ( "id" serial NOT NULL, "name" varchar NOT NULL, "per_batch" grams NOT NULL, PRIMARY KEY ("id") )"#
);
db.batch_execute(&create).await?;

let flour = ingredient::ActiveModel {
    name: set("Flour"),
    per_batch: set(500),
    ..Default::default()
}
.insert(&db)
.await?;
assert_eq!(flour.per_batch, 500);

let refused = ingredient::ActiveModel {
    name: set("Air"),
    per_batch: set(0),
    ..Default::default()
}
.insert(&db)
.await
.expect_err("the domain refuses zero grams");
assert!(matches!(refused.sql_error(), Some(SqlError::CheckConstraintViolation(_))));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A domain over an enum is read and written as the enum; PostgreSQL resolves no
comparison operator for such a domain, so pgorm compares its column cast to
the enum. PostgreSQL cannot build a foreign key on such a column, and [Schema
generation](crate::guide::schema) refuses a relation over one.

## Network addresses, bytes and vectors

`IpNetwork` is an `inet` (an address, with an optional network mask) and
`MacAddress` a `macaddr`; both are in the prelude. `Vec<u8>` is `bytea`, raw
bytes. `Vector` is the `vector` type of the [pgvector] extension, for
embeddings, and needs the extension installed in the database.

```rust
mod till {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "till")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub address: IpNetwork,
        pub hardware: MacAddress,
        pub receipt_logo: Vec<u8>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("values_network").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
# db.batch_execute(&pgorm::Schema::new().create_table_from_entity(till::Entity)?.to_string()).await?;

let front = till::ActiveModel {
    address: set("10.0.4.17/24".parse::<IpNetwork>()?),
    hardware: set(MacAddress::new([0x02, 0x00, 0x5e, 0x10, 0x00, 0x01])),
    receipt_logo: set(vec![0x89, b'P', b'N', b'G']),
    ..Default::default()
}
.insert(&db)
.await?;

assert_eq!(front.address.to_string(), "10.0.4.17/24");
assert_eq!(front.hardware.to_string(), "02:00:5E:10:00:01");
assert_eq!(front.receipt_logo, [0x89, b'P', b'N', b'G']);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Newtypes

A newtype over any of these types derives [`ValueType`](derive@crate::ValueType)
and then goes wherever its inner type goes: as a field, as a bound value, as
a decoded column. [Entities](crate::guide::entities) shows one. A type stored as
JSON derives `JsonValueType` instead, and a range type the schema creates
derives `CreatedRange`.

## Custom reads and writes

Sometimes a column needs SQL around it on its way in or out, because pgorm
has no Rust type that reads it directly. Four field attributes say so:

- `select_as = "type"` reads the column cast to a type, and `save_as =
  "type"` writes each value cast to one. The type is SQL, written as a
  string.
- `select_with = f` reads the column through a function `fn f(Expr) ->
  SimpleExpr`, and `save_with = f` writes each value through one, beside
  `save_array_with = g` for the arrays that `eq_any` and `ne_all` compare the
  column with.

The bakery keeps the days of the week it opens as a `bit(7)`, Monday first,
and the program reads and writes it as an integer:

```rust
mod opening {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "opening")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub bakery_id: i32,
        #[pgorm(column_type = ColumnType::Bit(Some(7)), select_as = "bigint", save_as = "bit(7)")]
        pub days: i64,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("values_custom").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
# db.batch_execute(&pgorm::Schema::new().create_table_from_entity(opening::Entity)?.to_string()).await?;

// Monday to Saturday.
let sunrise = opening::ActiveModel {
    bakery_id: set(1),
    days: set(0b111_1110),
}
.insert(&db)
.await?;
assert_eq!(sunrise.days, 0b111_1110);

let (sql, _) = opening::Entity::find()
    .filter(opening::Column::Days.eq(0b111_1111_i64))
    .build();
assert_eq!(
    sql,
    r#"SELECT "opening"."bakery_id", CAST("opening"."days" AS bigint) FROM "opening" WHERE "opening"."days" = CAST($1::int8 AS bit(7))"#
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

The functions are for a conversion a fixed type cannot say, such as a type
whose name is only known at run time, or one whose name needs quoting:

```rust
mod shelf {
    use pgorm::entity::prelude::*;
    use pgorm::pgorm_query::{SimpleExpr, TypeName};

    fn label() -> TypeName {
        TypeName::new(Name::runtime("Shelf Label"))
    }

    pub fn read_label(column: Expr) -> SimpleExpr {
        column.cast_as(Name::runtime("text"))
    }

    pub fn write_label(value: Expr) -> SimpleExpr {
        value.cast_as_type(label())
    }

    pub fn write_labels(values: Expr) -> SimpleExpr {
        values.cast_as_type(label().array())
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "shelf")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(
            column_type = ColumnType::named("Shelf Label"),
            select_with = read_label,
            save_with = write_label,
            save_array_with = write_labels
        )]
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# fn main() {
use pgorm::entity::prelude::*;

let (sql, _) = shelf::Entity::find()
    .filter(shelf::Column::Label.eq_any(["top", "middle"]))
    .build();
assert_eq!(
    sql,
    r#"SELECT "shelf"."id", CAST("shelf"."label" AS text) FROM "shelf" WHERE "shelf"."label" = ANY(CAST($1::text[] AS "Shelf Label"[]))"#
);
# }
```

[jiff]: https://docs.rs/jiff
[rust_decimal]: https://docs.rs/rust_decimal
[pgvector]: https://github.com/pgvector/pgvector
