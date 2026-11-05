# Values and types

Every column holds a value of some PostgreSQL type, and every field of a
record is the JavaScript type that stands for it. This chapter goes through
the types pgorm carries: what each one is in JavaScript, what it is in
PostgreSQL, and anything that travels between the two in a way you should
know about. It is the JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/12-values.md).

One rule runs through all of it: a value reaches JavaScript exactly or not at
all. A value that cannot is refused with a `DecodeError` or a
`ConstructionError`, never rounded, truncated or turned into a string.

## The types at a glance

A model column's *kind* decides what its field reads as, as
[Entities](03-entities.md) showed. Rust takes the column's type from the
field's Rust type; JavaScript has one type for every number, so the kind is
named in the declaration. The full vocabulary is:

| Kind | PostgreSQL | JavaScript |
| --- | --- | --- |
| `bool` | `boolean` | `boolean` |
| `i16`, `i32` | `smallint`, `integer` | `number` |
| `i64` | `bigint` | `bigint`, never a `number` |
| `f32`, `f64` | `real`, `double precision` | `number` |
| `decimal` | `numeric` | `Decimal` |
| `text` | `text`, `varchar`, `bpchar`, `name` | `string` |
| `bytes` | `bytea` | `Uint8Array` |
| `date`, `time`, `datetime` | `date`, `time`, `timestamp` | `Temporal.PlainDate`, `PlainTime`, `PlainDateTime` |
| `datetime_utc` | `timestamptz` | `Temporal.Instant` |
| `interval` | `interval` | `Interval` |
| `uuid` | `uuid` | `Uuid` |
| `json` | `json`, `jsonb` | JSON values; an integer past 2^53 − 1 as a `bigint` |
| an enum's `TypeName` | the enum | its label, a `string` |
| `array: true` | an array of the kind's type | an array, `null` for a NULL item |
| a range or multirange kind, `int4range` and the rest | that type | `Range`, `Multirange` |
| a `CreatedRange` | a range type the schema creates | `Range` |
| `ipnetwork`, `mac_address` | `inet` and `cidr`, `macaddr` | `string`, a six-byte `Uint8Array` |
| `vector` | pgvector's `vector` | `Float32Array` |
| `nullable: true` | the type, nullable | the type or `null` |

A `money` column and a column typed by a domain take their kind from what
they hold, and are covered below. A query that reads columns no model
declares, such as SQL text run on a pool, decodes each one the same way, by
the type the server describes it with.

## The `Value` container

When pgorm binds a parameter it carries it as one of its value variants, each
of a kind. JavaScript infers the kind of a plain value where its type leaves
one answer: a boolean is a `bool`, a safe-integer `number` an `i64`, any other
number an `f64`, a `bigint` an `i64`, a string `text`, a plain object `json`,
a Temporal value or one of the module's classes its own kind. A
[`Value`](../lib/index.d.ts) declares the kind where inference cannot: an
integer's width, a typed SQL NULL, JSON's `null`, an empty array, a range.
Rust's `Value` is the same container, and every Rust value converts into it
through its type; a JavaScript `Value` is made with its kind named:

```ts
import assert from "node:assert/strict";
import { ConstructionError, Decimal, Value } from "@necessary/pgorm";

const price = new Value(new Decimal("4.50"));
assert.equal(price.kind, "decimal");
assert.equal(String(price.value), "4.50");

// SQL NULL of its own kind, and JSON's null, which is not SQL NULL.
assert.ok(Value.null("text").isNull);
assert.equal(Value.null("text").kind, "text");
assert.equal(Value.json(null).isNull, false);

// A plain number infers as an int8; a Value says otherwise.
assert.equal(new Value(5).kind, "i64");
assert.equal(new Value(5, "i16").kind, "i16");
assert.throws(() => new Value(70000, "i16"), ConstructionError);
```

A value is checked as it is made, so one its kind cannot hold exactly never
reaches a statement. A built statement's `inspect()` returns its values as
`Value`s beside its SQL, as the [query builder](11-query-builder.md) chapter
shows. A row read with `{ tagged: true }` gives each column as a `Value` of
the kind it decoded as, which tells an `int2` from an `int8` and SQL NULL
from JSON's `null`.

## Dates and times

Dates and times are Temporal's, not `Date`'s. A `Date` holds milliseconds and
one meaning, an instant, where PostgreSQL has microseconds and four types.
Each PostgreSQL type has the Temporal type that holds exactly what it does,
as each has a jiff type in Rust:

- `Temporal.PlainDate` is a calendar date, `date`.
- `Temporal.PlainTime` is a time of day, `time`.
- `Temporal.PlainDateTime` is a date and time with no time zone, `timestamp`.
- `Temporal.Instant` is an instant, `timestamptz`. PostgreSQL stores the
  instant and no zone, and so does an `Instant`.

The bakery's bakers work shifts. A shift falls on a date, starts at a time of
day, and records the instant the baker clocked in:

```ts
import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";

await pool.execute(
  `CREATE TABLE shift (id serial PRIMARY KEY, baker_id integer NOT NULL, "on" date NOT NULL,
     starts time NOT NULL, planned timestamp NOT NULL, clocked_in timestamptz)`,
);
const Shift = model("shift", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    bakerId: column("i32", { name: "baker_id" }),
    on: column("date"),
    starts: column("time"),
    planned: column("datetime"),
    clockedIn: column("datetime_utc", { name: "clocked_in", nullable: true }),
  },
});

const early = await Shift.insert({
  bakerId: 1,
  on: Temporal.PlainDate.from("2026-11-06"),
  starts: Temporal.PlainTime.from("05:30"),
  planned: Temporal.PlainDateTime.from("2026-11-06T05:30"),
  clockedIn: Temporal.Instant.from("2026-11-06T04:28:00Z"),
}).returning().one(pool);

assert.equal(early.on.toString(), "2026-11-06");
assert.equal(early.starts.toString(), "05:30:00");
assert.equal(early.clockedIn?.toString(), "2026-11-06T04:28:00Z");

// Dates compare as dates.
const thisWeek = await Shift.find()
  .where(Shift.col("on").between(Temporal.PlainDate.from("2026-11-02"), Temporal.PlainDate.from("2026-11-08")))
  .count(pool);
assert.equal(thisWeek, 1);
```

A `timestamptz` comes back as the instant it was, whatever the session's time
zone, which shapes only its text. To show it in a zone, convert it in
JavaScript, `at.toZonedDateTimeISO("Europe/Oslo")`, where the zone rules are
explicit.

JavaScript refuses what Rust's types rule out when the program compiles. A
`Date` parameter is refused, rather than have its meaning guessed, and so is
a `ZonedDateTime`, rather than have its zone dropped. A `PlainDateTime` binds
only as a `timestamp` and an `Instant` only as a `timestamptz`. A value with a
digit below the microsecond is refused rather than truncated; round it first
with `instant.round({ smallestUnit: "microsecond" })`:

```ts
import assert from "node:assert/strict";
import { ConstructionError } from "@necessary/pgorm";

const precise = Temporal.Instant.from("2026-11-06T04:28:00.0000001Z");
await assert.rejects(pool.one("SELECT $1::timestamptz AS at", [precise]), ConstructionError);

const rounded = precise.round({ smallestUnit: "microsecond" });
const row = await pool.one("SELECT $1::timestamptz AS at", [rounded]);
assert.ok(row.at instanceof Temporal.Instant);
assert.equal(row.at.toString(), "2026-11-06T04:28:00Z");

await assert.rejects(pool.one("SELECT $1::timestamptz AS at", [new Date() as never]), /a Date is refused/);
```

`timetz` is refused when it is read: no Temporal type is a time of day at a
fixed offset. Select it cast to `time` or `text`.

## Numbers

Integers and floats map to `number`, except `bigint`, which reads as a
JavaScript `bigint` so that no integer past 2^53 is rounded. `numeric` maps to
`Decimal`, which is exact: `4.50` is stored as `4.50` and read back as
`4.50`, with its scale. A `Decimal` is made from a string or a `bigint`,
never from a `number`, which already holds `0.1` inexactly, and it refuses to
become a number implicitly.

Arithmetic the database does on a `numeric` stays exact, so a sum of prices
is the sum to the cent. JavaScript's `Decimal` holds a value and its text, and
does no arithmetic, where Rust adds `Decimal`s in the program; let the
database do the sums, or hand the text to a decimal library:

```ts
import assert from "node:assert/strict";
import { call, select } from "@necessary/pgorm";

const total = OrderLine.col("price").mul(OrderLine.col("quantity"));

const lines = await pool.query(
  select(total.as("total"))
    .from(OrderLine.table)
    .where(OrderLine.col("orderId").eq(1))
    .orderBy(OrderLine.col("id").asc()),
);
assert.deepEqual(lines.map((line) => String(line.total)), ["9.00", "3.20"]);

const order = await pool.one(
  select(call("sum", total).as("total")).from(OrderLine.table).where(OrderLine.col("orderId").eq(1)),
);
assert.equal(String(order.total), "12.20");
```

A `Decimal` holds up to 28 digits after the point, in a 96-bit coefficient.
Give a `numeric` column a precision and scale within that, such as
`decimal(10, 2)` for prices, since a value beyond it is refused as it is read
rather than rounded.

## Money

PostgreSQL's `money` stores an amount in the currency of the server's
`lc_monetary` setting. Rust reads and writes it as `numeric`, through a
`Decimal` field declared `ColumnType::Money`: the column is read cast to
`numeric` and each value written cast to `money`.

**Not available in JavaScript yet:** a model column of type `money`. A model
column has no money kind: a `decimal` column over `money` is refused, binding
with a `ConstructionError` and decoding with a `DecodeError`, and a raw query
that selects a `money` column cannot decode it either. Cast it to `numeric`
where it is read, and cast each value to `money` where it is written:

```ts
import assert from "node:assert/strict";
import { DecodeError, Decimal } from "@necessary/pgorm";

await pool.execute("CREATE TABLE tip_jar (id serial PRIMARY KEY, amount money NOT NULL)");
await pool.execute("INSERT INTO tip_jar (amount) VALUES ($1::numeric::money)", [new Decimal("12.345")]);

// Rounded to the currency's two places.
const tips = await pool.one("SELECT amount::numeric AS amount FROM tip_jar");
assert.equal(String(tips.amount), "12.35");

await assert.rejects(pool.query("SELECT amount FROM tip_jar"), DecodeError);
```

## JSON

A `json` column holds any JSON value, as JavaScript's own values: objects,
arrays, strings, numbers, booleans and `null`. A `json` column keeps the text
as written; a `jsonb` column stores it decomposed, which is what PostgreSQL
indexes and searches, and both are the `json` kind. An integer past
`Number.MAX_SAFE_INTEGER` reads as a `bigint`, exactly, and a number with no
exact binary form is a `DecodeError`.

`contains` is `@>`, which asks whether the `jsonb` holds the given JSON. A
plain object binds as JSON:

```ts
import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";

await pool.execute("CREATE TABLE profile (baker_id integer PRIMARY KEY, contact jsonb NOT NULL, extra json NOT NULL)");
const Profile = model("profile", {
  columns: {
    bakerId: column("i32", { name: "baker_id", primaryKey: true }),
    contact: column("json"),
    extra: column("json"),
  },
});

const ada = {
  bakerId: 1,
  contact: { phone: "+47 400 00 001", languages: ["nb", "en"] },
  extra: { apron: "L" },
};
await Profile.insert(ada).execute(pool);

const english = await Profile.find()
  .where(Profile.col("contact").contains({ languages: ["en"] }))
  .all(pool);
assert.deepEqual(english, [ada]);
```

Rust gives a JSON document of a fixed shape a type of its own, a struct
deriving `JsonValueType`, which is stored as its JSON and read back as the
struct. A JavaScript object already is its JSON, so a record's field is the
object itself.

**Not available in JavaScript yet:** declaring a JSON column's shape. A model
types every `json` field as any JSON value, so the declarations cannot type
`contact.phone` as a `string`, and nothing checks a document's shape as it is
read; narrow it in the program.

## Arrays

`array: true` makes a column an array of its kind. Rust has no arrays of its
network types or of `vector`; JavaScript declares and reads them, an `inet[]`
as an array of strings. `bytes` is not an array: it is `bytea`. An array
compares as a whole with `eq`, and the array operators ask about its
elements: `contains` is `@>` (holds every element of the value),
`containedBy` is `<@`, and `overlaps` is `&&` (shares an element). Rust's
`includes` is JavaScript's `contains`. An array of an enum is an array of the
enum's type, and its labels are cast to it.

The other direction, a scalar column against a list of values, is
`eq(any(list))`, which binds the whole list as one array parameter, so a
statement has the same SQL however long the list is. Rust spells it
`eq_any`:

```ts
import assert from "node:assert/strict";
import { any, column, model } from "@necessary/pgorm";

await pool.execute(
  "CREATE TABLE menu (id serial PRIMARY KEY, name varchar NOT NULL, tags varchar[] NOT NULL, flavors flavor[] NOT NULL)",
);
const Menu = model("menu", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    name: column("text"),
    tags: column("text", { array: true }),
    flavors: column(flavor, { array: true, values: ["chocolate", "vanilla", "lemon", "carrot"] }),
  },
});

await Menu.insertMany([
  { name: "Breakfast", tags: ["morning", "coffee"], flavors: ["lemon", "vanilla"] },
  { name: "Party", tags: ["evening"], flavors: ["chocolate"] },
]).execute(pool);

const withCoffee = await Menu.select("name").where(Menu.col("tags").contains(["coffee"])).all(pool);
assert.deepEqual(withCoffee, [{ name: "Breakfast" }]);

const chocolate = Menu.select("name").where(Menu.col("flavors").overlaps(["chocolate", "carrot"]));
assert.equal(
  chocolate.inspect().sql,
  `SELECT "menu"."name" FROM "menu" WHERE "menu"."flavors" && CAST($1::text[] AS public.flavor[])`,
);
assert.deepEqual(await chocolate.all(pool), [{ name: "Party" }]);

const named = Menu.select("name").where(Menu.col("name").eq(any(["Party", "Picnic"])));
assert.equal(named.inspect().sql, `SELECT "menu"."name" FROM "menu" WHERE "menu"."name" = ANY($1)`);
assert.deepEqual(await named.all(pool), [{ name: "Party" }]);
```

PostgreSQL arrays carry no dimension in their type, but pgorm reads one
dimension: an array is a list. A plain array parameter infers as an array of
the one kind its items infer as; an empty one has no kind to infer, and is
written `Value.array(kind, [])`.

## Enums

A model column of an enum names the enum's type with a `TypeName`, which
names its schema, and lists its labels ([Entities](03-entities.md) declares
one). pgorm writes each value cast to the enum's type, so a comparison
resolves:

```ts
import assert from "node:assert/strict";

const { sql, values } = Cake.select("name").where(Cake.col("flavor").eq("lemon")).inspect();
assert.equal(
  sql,
  `SELECT "cake"."name" FROM "cake" WHERE "cake"."flavor" = CAST($1::text AS public.flavor)`,
);
assert.deepEqual(values.map((value) => value.value), ["lemon"]);
```

A Rust entity reads an enum column cast to `text`. A JavaScript model reads
it as it is, and checks as it decodes that the column is of the enum the model
names and that its label is one the model lists; another schema's enum of the
same name, or a label the model does not list, is a `DecodeError`. An enum
Rust stores another way, as integers or as text, is in JavaScript a column of
that kind, `i16` or `text`, with no cast.

## UUIDs

A `uuid` column is the `uuid` kind, and reads as a `Uuid`, which holds its
lower-case hyphenated text. A key the database generates is a column with
`DEFAULT uuidv7()`, PostgreSQL 18's time-ordered UUIDs, which index well
because each is greater than the last:

```ts
import assert from "node:assert/strict";
import { column, Decimal, model, Uuid } from "@necessary/pgorm";

await pool.execute("CREATE TABLE voucher (code uuid PRIMARY KEY DEFAULT uuidv7(), value numeric NOT NULL)");
const Voucher = model("voucher", {
  columns: {
    code: column("uuid", { primaryKey: true, default: true }),
    value: column("decimal"),
  },
});

const first = await Voucher.insert({ value: new Decimal("10") }).returning("code").one(pool);
const second = await Voucher.insert({ value: new Decimal("10") }).returning("code").one(pool);

assert.ok(first.code instanceof Uuid);
// The version digit, and the order: a later v7 UUID sorts after an earlier one.
assert.equal(first.code.toString()[14], "7");
assert.ok(first.code.toString() < second.code.toString());
```

A `Uuid` compares with `equals`. It has no methods of its own beyond its text,
where Rust's `Uuid` reports its version and orders as a value; its text,
fixed-length lower-case hexadecimal, sorts as the value does.

## Intervals

An [`Interval`](../lib/index.d.ts) is PostgreSQL's `interval`: a number of
months, of days and of microseconds, each with its own sign, kept apart
because a month has no fixed number of days and a day no fixed number of
microseconds across a daylight-saving change. pgorm never turns one field
into another, so one month and thirty days are two different values. It is
the module's own class rather than a `Temporal.Duration`, whose fields must
share one sign and so cannot hold `1 mon -2 days`. `Interval.from` converts a
`Duration` or an ISO 8601 text, and `toDuration()` converts back where the
signs agree; a `Duration` also binds directly as an interval:

```ts
import assert from "node:assert/strict";
import { column, Interval, model } from "@necessary/pgorm";

await pool.execute(
  "CREATE TABLE recipe (id serial PRIMARY KEY, name varchar NOT NULL, proving interval NOT NULL, keeps_for interval NOT NULL)",
);
const Recipe = model("recipe", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    name: column("text"),
    proving: column("interval"),
    keepsFor: column("interval", { name: "keeps_for" }),
  },
});

const HOUR = 3_600_000_000n;
const sourdough = await Recipe.insert({
  name: "Sourdough",
  proving: new Interval({ microseconds: 14n * HOUR }),
  keepsFor: new Interval({ days: 3 }),
}).returning().one(pool);

assert.equal(sourdough.proving.microseconds, 14n * HOUR);
assert.equal(sourdough.keepsFor.toString(), "P3D");
assert.equal(sourdough.proving.toDuration().toString(), "PT14H");

const slow = await Recipe.find()
  .where(Recipe.col("proving").gt(Temporal.Duration.from({ hours: 12 })))
  .count(pool);
assert.equal(slow, 1);

// Fields of different signs, which a Duration cannot hold.
assert.equal(new Interval({ months: 1, days: -2 }).toString(), "P1M-2D");
assert.throws(() => new Interval({ months: 1, days: -2 }).toDuration(), RangeError);
```

An `Interval`'s text is PostgreSQL's `iso_8601` interval style, each field
signed, where Rust's `Display` writes PostgreSQL's own style, `+3 days`. A
column restricted to some fields, such as `interval HOUR TO MINUTE`, is
declared as `interval`; the restriction is the table's.

## Ranges and multiranges

A range is a span of values of one type, such as the dates a seasonal cake is
on sale. PostgreSQL has six built-in range types, and pgorm carries each as a
[`Range`](../lib/index.d.ts) of the type its bounds read as:

| Kind | JavaScript |
| --- | --- |
| `int4range`, `int8range` | `Range<number>`, `Range<bigint>` |
| `numrange` | `Range<Decimal>` |
| `daterange` | `Range<Temporal.PlainDate>` |
| `tsrange`, `tstzrange` | `Range<Temporal.PlainDateTime>`, `Range<Temporal.Instant>` |

A range is either empty or has two bounds, each included, excluded or absent.
`new Range(lower, upper, bounds)` makes one, `bounds` `"[)"` unless it says
otherwise, `null` on a side for no bound, and `Range.empty()` the empty
range. Rust converts the standard library's ranges, `a..b` and `a..=b`. A
[`Multirange`](../lib/index.d.ts) is a set of ranges, `datemultirange` for
dates. The column operators of [Arrays](#arrays) work on ranges too
(`contains`, `containedBy`, `overlaps`), beside `strictlyLeftOf`,
`strictlyRightOf`, `notExtendRightOf`, `notExtendLeftOf` and `adjacentTo`:

```ts
import assert from "node:assert/strict";
import { column, model, Range } from "@necessary/pgorm";

await pool.execute(
  "CREATE TABLE season (id serial PRIMARY KEY, cake varchar NOT NULL, on_sale daterange NOT NULL, batch_sizes int4range NOT NULL)",
);
const Season = model("season", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    cake: column("text"),
    onSale: column("daterange", { name: "on_sale" }),
    batchSizes: column("int4range", { name: "batch_sizes" }),
  },
});

const date = (text: string) => Temporal.PlainDate.from(text);
const gingerbread = await Season.insert({
  cake: "Gingerbread",
  onSale: new Range(date("2026-11-15"), date("2027-01-07")),
  batchSizes: new Range(12, 48, "[]"),
}).returning().one(pool);

// A discrete range comes back in PostgreSQL's canonical form: [12,49).
assert.equal(gingerbread.batchSizes.toString(), "[12,49)");
assert.deepEqual([gingerbread.batchSizes.lower, gingerbread.batchSizes.upper], [12, 49]);

// What is on sale over the Christmas week?
const christmas = new Range(date("2026-12-21"), date("2026-12-28"));
const onSale = await Season.select("cake").where(Season.col("onSale").overlaps(christmas)).all(pool);
assert.deepEqual(onSale, [{ cake: "Gingerbread" }]);
```

A model column takes a `Range` as it stands, its kind declared. A range bound
as a parameter of its own names its kind with a `Value`, since its bounds do
not say which range type it is: `[1, 5)` might be an `int4range` or an
`int8range`. A range given bare is refused:

```ts
import assert from "node:assert/strict";
import { ConstructionError, Multirange, Range, Value } from "@necessary/pgorm";

await assert.rejects(pool.one("SELECT $1 AS span", [new Range(1, 5) as never]), ConstructionError);

const empty = await pool.one("SELECT $1::int4range AS span", [new Value(new Range(5, 5), "int4range")]);
assert.ok(empty.span instanceof Range);
assert.equal(empty.span.isEmpty, true);

const merged = await pool.one("SELECT $1::int4multirange AS spans", [
  new Value(new Multirange([new Range(5, 8), new Range(1, 3), new Range(3, 4)]), "int4multirange"),
]);
assert.equal(String(merged.spans), "{[1,4),[5,8)}");
```

The empty range is a value of its own, never a pair of bounds: `[5,5)` reads
back as empty. A multirange comes back sorted, with overlapping and adjacent
ranges merged.

## Range types the schema creates

A schema can create a range type of its own, over a type no built-in range
covers, such as `double precision`. Rust names it once, in a newtype deriving
`CreatedRange`. JavaScript names it with a
[`CreatedRange`](../lib/index.d.ts) kind: its name, its schema, and the kind
of its subtype, which converts each bound. The type itself is created with the
DDL builders' `createType(..).asRange(..)` ([Schema
generation](14-schema.md)).

The bakery's ovens each hold a span of temperatures:

```ts
import assert from "node:assert/strict";
import { column, CreatedRange, createType, model, Range } from "@necessary/pgorm";

const createTemperature = createType("temperature").asRange("double");
assert.equal(createTemperature.inspect().sql, `CREATE TYPE "temperature" AS RANGE (SUBTYPE = double precision)`);
await pool.execute(createTemperature);
await pool.execute("CREATE TABLE oven (id serial PRIMARY KEY, name varchar NOT NULL, heat temperature NOT NULL)");

const temperature = new CreatedRange("temperature", "f64", { schema: "public" });
const Oven = model("oven", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    name: column("text"),
    heat: column(temperature),
  },
});

const deck = await Oven.insert({ name: "Deck oven", heat: new Range(180, 250.5) }).returning().one(pool);
assert.equal(deck.heat.toString(), "[180,250.5)");
assert.equal(
  Oven.insert({ name: "Deck oven", heat: new Range(180, 250.5) }).inspect().sql,
  `INSERT INTO "oven" ("name", "heat") VALUES ($1, CAST($2::text AS public.temperature))`,
);

const hotEnough = await Oven.find().where(Oven.col("heat").overlaps(new Range(240, 260))).count(pool);
assert.equal(hotEnough, 1);
```

A value of a created range type travels as its text, cast to the type by
name, and is read back from its binary form. A range type pgorm creates is
continuous whatever its subtype, since its canonical function would have to
be written in C. The multirange PostgreSQL creates beside it is a
`CreatedMultirange`, named by its own name. The driver cannot read such a
multirange's binary form, so a model reads a column of one cast to `text`, as
Rust's entities do, and reads that text as a `Multirange`; SQL of your own
casts it to `text` the same way.

## Domains

A domain is a type with constraints: `CREATE DOMAIN grams AS integer CHECK
(VALUE > 0)`. Its values travel as the type it is over, so a model column of a
domain has that type's kind. Rust also says the column is the domain, with
`ColumnType::Domain`, so that schema generation writes it; a JavaScript model
declares no column types for DDL ([Schema generation](14-schema.md)), so the
kind of what the domain holds is all it says. The domain's checks then hold
every write:

```ts
import assert from "node:assert/strict";
import { CheckViolationError, column, createDomain, domainValue, model } from "@necessary/pgorm";

const grams = createDomain("grams", "integer").check(domainValue().gt(0));
assert.equal(grams.inspect().sql, `CREATE DOMAIN "grams" AS integer CHECK (VALUE > 0)`);
await pool.execute(grams);
await pool.execute("CREATE TABLE ingredient (id serial PRIMARY KEY, name varchar NOT NULL, per_batch grams NOT NULL)");

const Ingredient = model("ingredient", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    name: column("text"),
    perBatch: column("i32", { name: "per_batch" }),
  },
});

const flour = await Ingredient.insert({ name: "Flour", perBatch: 500 }).returning().one(pool);
assert.equal(flour.perBatch, 500);

await assert.rejects(
  Ingredient.insert({ name: "Air", perBatch: 0 }).execute(pool),
  (error: unknown) => error instanceof CheckViolationError && error.datatype === "grams",
);
```

A domain over an enum is read and written as the enum. PostgreSQL resolves no
comparison operator for such a domain, so Rust compares its column cast to the
enum, and refuses a relation over one in schema generation, since PostgreSQL
cannot build a foreign key on it.

**Not available in JavaScript yet:** comparing a declared model's column of a
domain over an enum. Rust's entity declares the column as the domain, and
compares it cast to the enum; a JavaScript model's column names the enum
alone, so a comparison on it is the server's `DatabaseError`, *operator does
not exist*. A [registered entity](24-registered-entities.md) compares such a
column as Rust does, and so does a pipeline's `comparesAs` ([The
pipeline](12-pipeline.md)).

## Network addresses, bytes and vectors

`ipnetwork` is an `inet` or `cidr`, an address with an optional network mask,
read and written as its text; `mac_address` is a `macaddr`, six bytes in a
`Uint8Array`. `bytes` is `bytea`, raw bytes in a `Uint8Array`. Rust gives
the first two types of their own, `IpNetwork` and `MacAddress`:

```ts
import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";

await pool.execute(
  "CREATE TABLE till (id serial PRIMARY KEY, address inet NOT NULL, hardware macaddr NOT NULL, receipt_logo bytea NOT NULL)",
);
const Till = model("till", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    address: column("ipnetwork"),
    hardware: column("mac_address"),
    receiptLogo: column("bytes", { name: "receipt_logo" }),
  },
});

const front = await Till.insert({
  address: "10.0.4.17/24",
  hardware: new Uint8Array([0x02, 0x00, 0x5e, 0x10, 0x00, 0x01]),
  receiptLogo: new Uint8Array([0x89, 0x50, 0x4e, 0x47]),
}).returning().one(pool);

assert.equal(front.address, "10.0.4.17/24");
assert.deepEqual([...front.hardware], [0x02, 0x00, 0x5e, 0x10, 0x00, 0x01]);
assert.deepEqual([...front.receiptLogo], [0x89, 0x50, 0x4e, 0x47]);
```

`vector` is the `vector` type of the [pgvector] extension, for embeddings,
and needs the extension installed in the database. It reads and writes as a
`Float32Array`, and a value of one is never inferred, since a `Float32Array`
could be several things:

```ts
import assert from "node:assert/strict";
import { Value } from "@necessary/pgorm";

const embedding = new Value(new Float32Array([0.25, 0.5, 1]), "vector");
assert.equal(embedding.kind, "vector");
assert.deepEqual([...(embedding.value as Float32Array)], [0.25, 0.5, 1]);
```

## Newtypes

A Rust newtype over any of these types derives `ValueType` and then goes
wherever its inner type goes, a type the compiler keeps apart from the one it
wraps. JavaScript has no newtypes: a field reads as its kind's type, and a
program that wants grams kept apart from other integers brands the type in
TypeScript itself.

## Custom reads and writes

Sometimes a column needs SQL around it on its way in or out, because pgorm
has no type that reads it directly. Rust's entities say so with four field
attributes: `select_as` reads the column cast to a type and `save_as` writes
each value cast to one, and `select_with` and `save_with` read and write
through functions of the column's expression.

**Not available in JavaScript yet:** custom reads and writes on a model
column. A model's column is read as it is and written as its kind, so a
column of a type pgorm cannot read, such as a `bit(7)` the program reads as an
integer, is read and written through SQL with its casts written out:

```ts
import assert from "node:assert/strict";

await pool.execute("CREATE TABLE opening (bakery_id integer PRIMARY KEY, days bit(7) NOT NULL)");

// Monday to Saturday.
await pool.execute("INSERT INTO opening (bakery_id, days) VALUES ($1, $2::int4::bit(7))", [1, 0b111_1110]);

const sunrise = await pool.one("SELECT days::int8 AS days FROM opening WHERE bakery_id = $1", [1]);
assert.equal(sunrise.days, 0b111_1110n);
```

[pgvector]: https://github.com/pgvector/pgvector
