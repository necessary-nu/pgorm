# Entities

An *entity* is a table, described so that pgorm can read and write it. In
Rust an entity is a module of types its derives write. In JavaScript it is a
*model*: a value that [`model`](../lib/models.d.ts) makes from the table's
name and an object of its columns. Everything that reads or writes the table
starts from it.

This chapter declares the bakery's models and goes through what a declaration
can say. The other chapters use these models. It is the JavaScript dialect of
the [Rust guide's chapter](../../docs/guide/02-entities.md), and each section
says where a model differs from a Rust entity.

## Declaring a model

Here is the bakery's `cake` table:

```ts
import assert from "node:assert/strict";
import { column, DataType, model, TypeName } from "@necessary/pgorm";

const flavor = new TypeName("flavor", { schema: "public" });

const Cake = model("cake", {
  columns: {
    id: column("i32", { primaryKey: true, autoIncrement: true }),
    name: column("text", { columnType: "varchar" }),
    flavor: column(flavor, { values: ["chocolate", "vanilla", "lemon", "carrot"] }),
    price: column("decimal", { columnType: new DataType("numeric", { precision: 10, scale: 2 }) }),
    glutenFree: column("bool", { name: "gluten_free" }),
    bakeryId: column("i32", { name: "bakery_id" }),
  },
});

assert.equal(Cake.name, "cake");
const columns = Object.values(Cake.describe().fields).map((field) => field.column);
assert.deepEqual(columns, ["id", "name", "flavor", "price", "gluten_free", "bakery_id"]);
```

Each key of `columns` is a *field*, the name the program uses, and each
[`column`](../lib/models.d.ts) says what the field holds. A field is named for
its column unless `name` gives the column's own, as `glutenFree` does for
`gluten_free`, or the model's `renameAll` names every column by a case style,
as Rust's `rename_all` does and by the same conversion: [`bakery.ts`](bakery.ts)
declares `renameAll: "snake_case"`, so its `bakeryId` is the column
`bakery_id`. `"camelCase"`, `"PascalCase"`, `"kebab-case"`,
`"SCREAMING_SNAKE_CASE"`, `"SCREAMING-KEBAB-CASE"`, `"lowercase"`,
`"UPPERCASE"`, `"title_case"` and `"mixed_case"` are the others; any else is
a `TypeError`. The model then offers everything Rust's `Entity` derive writes,
as methods of one value:

- `find()`, `select(..)` and `findByKey(..)` start a `SELECT`, decoded into
  *records*: plain objects keyed by field ([Reading rows](05-reading.md)).
- `insert`, `insertMany`, `update` and `delete` start the writes
  ([Writing rows](04-writing.md)).
- `col(field)` names a column in a condition or an ordering, as Rust's
  `Column` enum does: `Cake.col("glutenFree")`.
- `primaryKey` lists the key's fields, as Rust's `PrimaryKey` enum does, and
  `key(..)` is the condition matching one row by its key.
- `belongsTo`, `hasOne` and `hasMany` declare relations, and `graph()` starts
  the relational read ([Relations](07-relations.md)).
- `describe()` gives the declaration back as data.

Rust's `ActiveModel` has no counterpart here. A write takes a plain object of
the fields it sets, and a field the object leaves out is a column the
statement leaves out ([Writing rows](04-writing.md)).

The bakery's seven models, `Bakery`, `Baker`, `Cake`, `CakeBaker`,
`Customer`, `Order` and `OrderLine`, are declared the same way, with their
relations, in [`guide/bakery.ts`](bakery.ts), which every example in this
guide runs against.

Every option of a declaration is checked as it is made. An option `column`
does not know, or one of the wrong type, is a `TypeError`, and a combination
that cannot be used, such as a nullable key column, is a `ConstructionError`.
Rust refuses the same mistakes when the program compiles; JavaScript refuses
them when the module that declares the model loads, before any statement is
built:

```ts
import assert from "node:assert/strict";
import { column, ConstructionError } from "@necessary/pgorm";

// A misspelt option is never silently left out.
assert.throws(() => column("i32", { primary_key: true } as never), TypeError);

// A key column cannot hold NULL.
assert.throws(() => column("i32", { primaryKey: true, nullable: true }), ConstructionError);
```

TypeScript catches the misspelling sooner still, which is why the example
casts it to get past the type checker ([TypeScript](26-typescript.md)).

## Column kinds

A column's *kind* says how its values travel and what JavaScript value each
one is. It is a kind's name, or a `TypeName` for an enum, or a
`CreatedRange` or `CreatedMultirange` for a range type a schema created:

| Kind | PostgreSQL | JavaScript |
| --- | --- | --- |
| `"text"` | `text`, `varchar`, `bpchar`, `name` | `string` |
| `"i16"`, `"i32"`, `"i64"` | `smallint`, `integer`, `bigint` | `number`, `number`, `bigint` |
| `"f32"`, `"f64"` | `real`, `double precision` | `number` |
| `"bool"` | `boolean` | `boolean` |
| `"decimal"` | `numeric` | `Decimal` |
| `"date"`, `"time"`, `"datetime"` | `date`, `time`, `timestamp` | `Temporal.PlainDate`, `PlainTime`, `PlainDateTime` |
| `"datetime_utc"` | `timestamptz` | `Temporal.Instant` |
| `"uuid"` | `uuid` | `Uuid` |
| `"json"` | `json`, `jsonb` | a JSON value |
| `"bytes"` | `bytea` | `Uint8Array` |
| a `TypeName` | an enum | the label, a `string` |

`nullable: true` lets the column hold NULL, read as `null`, and
`array: true` makes it a one-dimensional array of its kind. [Values and
types](13-values.md) has the whole vocabulary, from intervals to ranges.

A kind is not a column type: `"text"` reads a `varchar` as readily as a
`text`, and `"decimal"` reads a `numeric` of any precision. The type matters
to the table the model writes, which takes the kind's own type unless
`columnType` names another, as Rust's `column_type` does: `"varchar"`, or a
`DataType` with a length or a precision, as the cake's price above. An array
column's `columnType` is its elements' type.

A model writes its own `CREATE TABLE`, as an entity does: `fromModel(model)`
hands the declaration to pgorm's own schema generation and returns the
`EntitySchema` a [registered entity](24-registered-entities.md)'s
`fromEntity` returns, its table, enum types, indexes and comments
([Schema generation](14-schema.md)). The bakery's customers are unique by
email:

```ts
import assert from "node:assert/strict";
import { fromModel } from "@necessary/pgorm";

assert.equal(
  fromModel(Customer).table.inspect().sql,
  `CREATE TABLE "customer" ( "id" serial NOT NULL, "name" varchar NOT NULL, "email" varchar NOT NULL, "notes" varchar, PRIMARY KEY ("id"), UNIQUE ("email") )`,
);
```

That is the statement Rust writes for the Rust guide's `customer` entity.
Declaring a model still sends nothing; `fromModel` only builds statements,
which the program runs with `execute`.

What a declaration says can be read back. `describe()` reports each field's
column, kind and options, and the table's keys:

```ts
import assert from "node:assert/strict";

const { primaryKey, uniqueKeys, fields } = Customer.describe();
assert.deepEqual(primaryKey, ["id"]);
assert.deepEqual(uniqueKeys, [{ name: null, fields: ["email"], nullsNotDistinct: false }]);
assert.equal(fields.notes?.kind, "text");
assert.equal(fields.notes?.nullable, true);
```

## Primary keys

A column marked `primaryKey` is part of the table's primary key, which is
every such column in declaration order. `findByKey` and `key` take the key as
an object of its fields, so a key of several columns reads the same as a key
of one. The bakery's `cake_baker` table records which baker bakes which cake,
keyed by the pair:

```ts
import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";

const CakeBaker = model("cake_baker", {
  columns: {
    cakeId: column("i32", { name: "cake_id", primaryKey: true }),
    bakerId: column("i32", { name: "baker_id", primaryKey: true }),
  },
});

assert.deepEqual(CakeBaker.primaryKey, ["cakeId", "bakerId"]);

// Bo bakes the Lemon Drizzle.
const row = await CakeBaker.findByKey({ cakeId: 2, bakerId: 2 }).optional(pool);
assert.deepEqual(row, { cakeId: 2, bakerId: 2 });
```

Rust's `find_by_id` takes a composite key as a tuple in the key's order;
JavaScript names each part. Rust also refuses an entity with no primary key;
a JavaScript model may have none, and then has no key for `findByKey` or
`key` to take.

A key the database generates is one an insert may leave out, and the model
says so. `autoIncrement: true` draws a one-column `"i16"`, `"i32"` or `"i64"`
key from the serial family, as Rust's one-column integer key is unless it
says `auto_increment = false`: every bakery model's `id` is one, written
`serial`. `generated: "byDefault"` marks a `GENERATED BY DEFAULT AS
IDENTITY` column, which an insert may also set, and `generated: "always"` a
`GENERATED ALWAYS AS IDENTITY` one, which no write may set; Rust writes
these `identity_by_default` and `identity`. A column with none of them, like
each of `cake_baker`'s, is one the program chooses.

A multi-tenant table keyed by `(tenant_id, id)` generates its `id` inside the
composite key, and an insert gives the tenant and gets the id back:

<!-- example: empty -->
```ts
import assert from "node:assert/strict";
import { column, fromModel, model } from "@necessary/pgorm";

const Note = model("note", {
  renameAll: "snake_case",
  columns: {
    tenantId: column("i32", { primaryKey: true }),
    id: column("i64", { primaryKey: true, generated: "always" }),
    body: column("text", { columnType: "varchar" }),
  },
});

const create = fromModel(Note).table;
assert.equal(
  create.inspect().sql,
  `CREATE TABLE "note" ( "tenant_id" integer NOT NULL, "id" bigint NOT NULL GENERATED ALWAYS AS IDENTITY, "body" varchar NOT NULL, CONSTRAINT "pk-note" PRIMARY KEY ("tenant_id", "id") )`,
);
await pool.execute(create);

const note = await Note.insert({ tenantId: 7, body: "Order more flour" }).returning().one(pool);
assert.deepEqual(note, { tenantId: 7, id: 1n, body: "Order more flour" });
```

A key whose last field is a range or multirange can be a PostgreSQL 18
temporal key: `withoutOverlaps: true` on that field, as Rust's
`primary_key, without_overlaps`, makes it the key's period, written last,
`WITHOUT OVERLAPS`, and no two rows with equal other key fields may hold
overlapping ranges. A period on a field but the key's last, or in a key of
one field, which PostgreSQL refuses, is a `ConstructionError`.

## Defaults, generated columns and keys

A column can take a default, be computed from the others, carry a comment,
be indexed, or be unique, each said as Rust's attribute of the same name
says it:

- `defaultValue: 1` gives a constant default, converted through the
  column's kind, and `defaultExpr: ..` an expression, such as
  `call("uuidv7")`. `default: true` says the table has a default the
  declaration does not state: an insert may leave the column out, but its
  table has no `DEFAULT` to write, so `fromModel` refuses it.
- `generatedStored: ..` and `generatedVirtual: ..` make a generated column,
  computed from an expression over the row's other columns, which the
  database stores or computes on read. No write names one: the model reports
  it `generated: "always"`.
- `uniqueKeys` lists the table's unique keys, each of one field or several,
  with an optional `name` and `nullsNotDistinct`, as Rust's `unique(..)`
  does. A key naming no field, a field the model lacks, or one field twice is
  a `ConstructionError`.
- `indexed: true` asks for an index on the column, and `comment` on a
  column or on the model becomes a `COMMENT ON`, statements of their own in
  the model's schema ([Schema generation](14-schema.md)).

PostgreSQL fills a column one way, so two of a default, stated or not, an
identity, a generation expression and `autoIncrement` on one column are a
`ConstructionError`, and an insert's type lets each of them be left out.

The bakery's order lines could carry their total as a stored generated
column, and allow each cake once per order:

<!-- example: empty -->
```ts
import assert from "node:assert/strict";
import { col, column, DataType, Decimal, fromModel, model } from "@necessary/pgorm";

const money = new DataType("numeric", { precision: 10, scale: 2 });
const OrderLine = model("order_line", {
  renameAll: "snake_case",
  columns: {
    id: column("i32", { primaryKey: true, autoIncrement: true }),
    orderId: column("i32"),
    cakeId: column("i32"),
    quantity: column("i32", { defaultValue: 1 }),
    price: column("decimal", { columnType: money }),
    total: column("decimal", { columnType: money, generatedStored: col("price").mul(col("quantity")) }),
  },
  uniqueKeys: [{ name: "one_line_per_cake", fields: ["orderId", "cakeId"] }],
});

const create = fromModel(OrderLine).table;
assert.equal(
  create.inspect().sql,
  `CREATE TABLE "order_line" ( "id" serial NOT NULL, "order_id" integer NOT NULL, "cake_id" integer NOT NULL, "quantity" integer NOT NULL DEFAULT 1, "price" decimal(10, 2) NOT NULL, "total" decimal(10, 2) NOT NULL GENERATED ALWAYS AS ("price" * "quantity") STORED, PRIMARY KEY ("id"), CONSTRAINT "one_line_per_cake" UNIQUE ("order_id", "cake_id") )`,
);
await pool.execute(create);

// The quantity takes its default, and the database computes the total.
const line = await OrderLine.insert({ orderId: 1, cakeId: 2, price: new Decimal("3.20") })
  .returning("quantity", "total")
  .one(pool);
assert.equal(line.quantity, 1);
assert.equal(line.total.toString(), "3.20");
```

## Relations

A relation is a value a model's `belongsTo`, `hasOne` or `hasMany` makes. It
names the model it starts at, the model it reaches, and the fields of each
end that are equal in a related pair of rows, matched in order:

- `belongsTo(target, { from, to })` is the side holding the foreign key:
  `from` names this model's fields and `to` the target's, an array of fields
  for a composite key. It reaches at most one row.
- `hasMany` and `hasOne` are the other side, from this model's key to the
  rows that point at it; `hasOne` reaches at most one.

```ts
import assert from "node:assert/strict";

const cakeBakery = Cake.belongsTo(Bakery, { from: "bakeryId", to: "id" });
const bakeryCakes = Bakery.hasMany(Cake, { from: "id", to: "bakeryId" });

assert.equal(cakeBakery.cardinality, "one");
assert.equal(bakeryCakes.cardinality, "many");
assert.deepEqual([cakeBakery.fromFields, cakeBakery.toFields], [["bakeryId"], ["id"]]);

const lemonDrizzle = await Cake.findByKey({ id: 2 }).one(pool);
const sunrise = await cakeBakery.find(lemonDrizzle).one(pool);
assert.equal(sunrise.name, "Sunrise");
```

A Rust entity lists its relations in its `Relation` enum, and its `Related`
implementations let a query name the related entity alone. A JavaScript
model holds no list of its relations: each relation is a value the program
keeps and passes where a relation is needed, which is why the bakery's are
exported beside its models, each named for the model it starts at and what
it reaches (`cakeBakery`, `bakeryCakes`). Two relations between the same two
models are simply two values. A `belongsTo` relation is also its table's
foreign key: `fromModel(model, { relations })` writes one for each
`belongsTo` relation it is given, and the relation takes the key's
`onDelete` and `onUpdate` actions (`"noAction"`, `"restrict"`, `"cascade"`,
`"setNull"`, `"setDefault"`) and its `fkName`, as Rust's does: the
bakery's `cakeBakerCake` deletes a cake's `cake_baker` rows with it.
`hasOne` and `hasMany` take none of them, a `TypeError`.

A many-to-many relation goes through a junction table: the cake's bakers are
reached through `cake_baker`, along `cakeCakeBakers` and then
`cakeBakerBaker`. [Relations](07-relations.md) reads along such a path.

**Not available in JavaScript yet:** a relation narrowed by a condition of
its own, which Rust's `on_condition` adds to a relation's join, and a
declared path through a junction that one call follows, as a Rust `Related`
implementation with a `via` does for `find_related`.

## Enums

A PostgreSQL enum column has a `TypeName` as its kind, naming the type and
its schema, and lists the type's labels in `values`. The cake's flavor is the
type `flavor` in the `public` schema:

```ts
import assert from "node:assert/strict";
import { ConstructionError, Decimal } from "@necessary/pgorm";

assert.equal(Cake.columns.flavor.kind.toString(), `"public"."flavor"`);
assert.deepEqual(Cake.columns.flavor.values, ["chocolate", "vanilla", "lemon", "carrot"]);

const carrot = await Cake.find().where(Cake.col("flavor").eq("carrot")).one(pool);
assert.equal(carrot.name, "Carrot Cake");

// A label the declaration does not list is refused before anything is sent.
assert.throws(
  () =>
    Cake.insert({
      name: "Banana Bread",
      // @ts-expect-error: "banana" is not one of the flavor's labels
      flavor: "banana",
      price: new Decimal("3.00"),
      glutenFree: false,
      bakeryId: 1,
    }),
  ConstructionError,
);
```

Here a model differs from a Rust `ActiveEnum` in three ways:

- **The labels are the values.** A record's flavor is the string `"carrot"`,
  and TypeScript types it as the union of the listed labels. There is no
  enum of variants to convert, so Rust's `rename_all` and `string_value`
  have nothing to rename.
- **The type names its schema,** `public` included. A result names each
  column's type with its schema, and the model checks it against the
  declaration, so a flavor read from another schema's enum is a
  `DecodeError`, never a label of the wrong type. A comparison and a write
  cast each label to the type, `CAST($1::text AS public.flavor)`.
- **Only PostgreSQL enums are enums.** Rust's `rs_type` and `db_type` store
  an enum as integers or as text. A JavaScript model declares such a column
  by its kind, `"i16"` or `"text"`, and the program maps its values itself.

`values` is optional: without it, a label is any string, and a record's type
says so.

## Hooks

A Rust entity can implement `ActiveModelBehavior`, whose hooks run around an
active model's `insert`, `update` and `delete`: `new` fills defaults, and
`before_save` and `before_delete` can refuse the write. A JavaScript model has
no active model, so its writes are the statements they build, and nothing
runs around them. Code that must run around a write is a function of the
program's own that does its work and then writes.

An application whose entities have hooks in Rust can register those
entities, and JavaScript's writes through them then run the hooks: a
refusal arrives as a `ConstructionError` carrying the hook's message
([Registered Rust entities](24-registered-entities.md)).

## The other derives

A Rust program declares other shapes beside an entity, each with a derive.
Here is what each one is in JavaScript:

| Rust derive | In JavaScript |
| --- | --- |
| `PartialModel` | `select(..fields)` on a model reads some of its fields into records ([Reading rows](05-reading.md)) |
| `FromQueryResult` | a row of any statement is a plain object keyed by column name ([Raw SQL](10-raw-sql.md)) |
| `Model` | a second `model(..)` over the same table, declaring some of its columns |
| `IntoActiveModel` | an object of the fields a form sets is already an insert's values, below |
| `ValueType` | no counterpart: a field holds its kind's JavaScript value |
| `JsonValueType` | a `"json"` column, read as a JSON value |
| `CreatedRange` | a `CreatedRange` kind ([Values and types](13-values.md)) |
| `Disjoint` | `disjoint(..)` ([Disjoint enums](15-disjoint.md)) |
| `OneOf` | `oneOf(..)` ([One-of references](16-one-of.md)) |
| `Column` | no counterpart: a field is named by its string |
| `EntityTrait` | `model(..)` takes names computed at run time, [below](#names-computed-at-run-time) |
| `SqlName`, `StaticName` | a name is a string, quoted wherever it is written ([The query builder](11-query-builder.md)) |

A Rust `ValueType` newtype keeps, say, grams of flour apart from other
integers at compile time. A field of kind `"i32"` is a `number`, and a
TypeScript program that wants such a distinction makes it in its own types.

**Not available in JavaScript yet:** a JSON column whose documents decode as
a type of the program's choosing, checked as they are read, which a Rust
`JsonValueType` gives. A `"json"` field reads as a JSON value, typed as one,
whatever shape its documents have.

Rust's `IntoActiveModel` turns a struct such as a sign-up form into an active
model with its fields set and the rest left out. A JavaScript form is already
that: an object holding some of the fields, and an insert of it leaves the
rest to their defaults:

<!-- example: empty -->
```ts
import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";

await pool.execute(
  `CREATE TABLE subscriber ( id serial NOT NULL, name varchar NOT NULL, email varchar NOT NULL, weekly bool NOT NULL DEFAULT TRUE, PRIMARY KEY (id), UNIQUE (email) )`,
);

const Subscriber = model("subscriber", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    name: column("text"),
    email: column("text"),
    weekly: column("bool", { default: true }),
  },
  uniqueKeys: [{ fields: "email" }],
});

// What the newsletter's sign-up form sends: two of the columns.
const form = { name: "Mo", email: "mo@example.com" };
const mo = await Subscriber.insert(form).returning().one(pool);
assert.equal(mo.email, "mo@example.com");
// The form left `weekly` out, so it took the column's default.
assert.equal(mo.weekly, true);
```

## Names computed at run time

A Rust entity's table and schema are fixed when it is written, unless a
hand-written `Entity` derives `EntityTrait` and computes them. A JavaScript
model is made when the program runs, so its names are whatever the program
computes then: a deployment that keeps each region in a schema of its own
declares its models from the region it was started for.

```ts
import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";

// The schema this deployment keeps its tables in, chosen at startup.
const region = "oslo";

const RegionalCake = model("cake", {
  schema: region,
  columns: {
    id: column("i32", { primaryKey: true }),
    name: column("text"),
  },
});

assert.equal(RegionalCake.find().inspect().sql, `SELECT "cake"."id", "cake"."name" FROM "oslo"."cake"`);
```

The same table read twice in one statement, as a join of a table to itself
needs, is the model read under an alias: `Cake.as("other")` qualifies every
column by `"other"`.
