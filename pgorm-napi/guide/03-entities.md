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
import { column, model, TypeName } from "@necessary/pgorm";

const flavor = new TypeName("flavor", { schema: "public" });

const Cake = model("cake", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    name: column("text"),
    flavor: column(flavor, { values: ["chocolate", "vanilla", "lemon", "carrot"] }),
    price: column("decimal"),
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
`gluten_free`. The model then offers everything Rust's `Entity` derive writes,
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

A kind is not a column type in DDL. Rust's field types also say how
[schema generation](14-schema.md) creates the table: `String` is `varchar`,
and `column_type` chooses another type, such as `decimal(10, 2)`. A
JavaScript model says nothing about how its table is made, so `"text"` reads
a `varchar` as readily as a `text`, and `"decimal"` reads a `numeric` of any
precision. Rust's `rename_all`, which renames every column by a case style,
has no counterpart either: each field that differs from its column says so
with `name`.

**Not available in JavaScript yet:** generating the `CREATE TABLE`, its keys,
indexes and comments from a declared model, which Rust's
`Schema::create_table_from_entity` does for an entity. A JavaScript program
creates its tables with the DDL builders of [Schema
generation](14-schema.md), or registers Rust entities and generates their
tables from them ([Registered Rust entities](24-registered-entities.md)).

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
says so. `default: true` marks a column the table fills when an insert leaves
it out, as a `serial` key's sequence does: every bakery model's `id` is one.
`generated: "byDefault"` marks a `GENERATED BY DEFAULT AS IDENTITY` column,
which an insert may also set, and `generated: "always"` a `GENERATED ALWAYS`
column, identity or computed, which no write may set. A column with neither,
like each of `cake_baker`'s, is one the program chooses; it is what Rust's
`auto_increment = false` says.

A multi-tenant table keyed by `(tenant_id, id)` generates its `id` inside the
composite key, and an insert gives the tenant and gets the id back:

<!-- example: empty -->
```ts
import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";

await pool.execute(
  `CREATE TABLE note ( tenant_id integer NOT NULL, id bigint NOT NULL GENERATED ALWAYS AS IDENTITY, body varchar NOT NULL, PRIMARY KEY (tenant_id, id) )`,
);

const Note = model("note", {
  columns: {
    tenantId: column("i32", { name: "tenant_id", primaryKey: true }),
    id: column("i64", { primaryKey: true, generated: "always" }),
    body: column("text"),
  },
});

const note = await Note.insert({ tenantId: 7, body: "Order more flour" }).returning().one(pool);
assert.deepEqual(note, { tenantId: 7, id: 1n, body: "Order more flour" });
```

A key whose last column is a range can be a PostgreSQL 18 temporal key,
`WITHOUT OVERLAPS`. That is a property of the table, which a model does not
create: the model reads and writes such a table as it does any other, and
the DDL builders create its key ([Schema generation](14-schema.md)).

## Defaults, generated columns and keys

A column can take a default, be computed from the others, or be unique. A
model needs to know only what changes its reads and writes:

- `default: true` says the table fills the column when an insert leaves it
  out, so the insert's type lets it be left out. The default itself, a
  constant or an expression such as `uuidv7()`, is the table's.
- `generated: "always"` says the database computes the column, stored or
  virtual, so no write names it. The expression is the table's.
- `uniqueKeys` lists the table's unique keys, each of one field or several,
  with an optional `name` and `nullsNotDistinct`, as Rust's `unique(..)`
  does. A key naming no field, a field the model lacks, or one field twice is
  a `ConstructionError`.

Rust's `default_value`, `default_expr`, `generated_stored`,
`generated_virtual`, `indexed` and `comment` also say how [schema
generation](14-schema.md) creates the column; in JavaScript that is the DDL
builders' to say.

The bakery's order lines could carry their total as a stored generated
column, and allow each cake once per order:

<!-- example: empty -->
```ts
import assert from "node:assert/strict";
import { column, Decimal, model } from "@necessary/pgorm";

await pool.execute(
  `CREATE TABLE order_line ( id serial NOT NULL, order_id integer NOT NULL, cake_id integer NOT NULL, quantity integer NOT NULL DEFAULT 1, price decimal(10, 2) NOT NULL, total decimal(10, 2) NOT NULL GENERATED ALWAYS AS (price * quantity) STORED, PRIMARY KEY (id), CONSTRAINT one_line_per_cake UNIQUE (order_id, cake_id) )`,
);

const OrderLine = model("order_line", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    orderId: column("i32", { name: "order_id" }),
    cakeId: column("i32", { name: "cake_id" }),
    quantity: column("i32", { default: true }),
    price: column("decimal"),
    total: column("decimal", { generated: "always" }),
  },
  uniqueKeys: [{ name: "one_line_per_cake", fields: ["orderId", "cakeId"] }],
});

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
models are simply two values. A relation's `on_delete` and `on_update` are
the foreign key's, so they belong to the table's DDL.

A relation narrowed by a condition of its own takes `onCondition`, as Rust's
`on_condition` does: a function given the relation's two ends, each the
model as a join names its table, returning a condition. It is ANDed with the
fields' equality, or ORed with it under `conditionType: "any"`, Rust's
`ConditionType::Any`. Rust's closure is given the two tables' names, and a
JavaScript one the two models answering to them, so `bakery.col("city")` is
qualified as the join names the bakery:

```ts
import assert from "node:assert/strict";

const cakeInOslo = Cake.belongsTo(Bakery, {
  from: "bakeryId",
  to: "id",
  onCondition: (_cake, bakery) => bakery.col("city").eq("Oslo"),
});
assert.equal(
  Cake.find().join(cakeInOslo).inspect().sql,
  `SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", "cake"."gluten_free", "cake"."bakery_id" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" AND "bakery"."city" = $1`,
);
assert.equal(cakeInOslo.conditionType, "all");
```

A many-to-many relation goes through a junction table: the cake's bakers are
reached through `cake_baker`, along `cakeCakeBakers` and then
`cakeBakerBaker`. Where Rust writes the cake's `Related<baker::Entity>` by
hand, with `via` the junction's relation to the cake, JavaScript declares the
same path with the model's `related`: `Cake.related(cakeBakerBaker, { via:
cakeCakeBakers })`, which the bakery exports as `cakeBakers`.
[Relations](07-relations.md) reads along such a path.

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
`before_save` and `before_delete` can refuse the write. A JavaScript model
takes its hooks as functions, `hooks: { beforeSave(values, insert),
afterSave(record, insert), beforeDelete(values), afterDelete(values) }`, each
optional and each free to return a promise. They run around the model's
record writes: `Model.active(values)` holds the fields one write sets, and
its `insert`, `update` and `delete` write one record by its primary key, as
Rust's active model writes itself.

`beforeSave` and `beforeDelete` give the values to write, and refuse the
write by throwing: nothing is sent, and the error reaches the caller as it
was thrown. `afterSave` gives the record the write returns. An `after` hook
that throws reports its error after the row is written, as Rust's does; run
the write in a transaction to undo it. Rust's `new`, which fills the defaults
an active model starts from, is the program's own here, as `beforeSave`'s
spread does below. The statement builders of [Writing rows](04-writing.md),
`insert`, `insertMany`, `update` and `delete` on the model, their returning
and try-insert terminals, and an update `from` or a delete `using`, run no
hook, as Rust's `Insert`, `Update` and `Delete` run none.

```ts
import assert from "node:assert/strict";
import { column, ConstructionError, model } from "@necessary/pgorm";

const Special = model("special", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    name: column("text"),
    note: column("text"),
  },
  hooks: {
    beforeSave(values) {
      if (!values.name) throw new ConstructionError("a special needs a name");
      return { note: "Ask at the counter", ...values };
    },
  },
});

await pool.execute("CREATE TABLE special (id serial PRIMARY KEY, name text NOT NULL, note text NOT NULL)");
const saved = await Special.active({ name: "Saturday cinnamon buns" }).insert(pool);
assert.equal(saved.note, "Ask at the counter");

await assert.rejects(Special.active({ name: "" }).insert(pool), /a special needs a name/);
assert.deepEqual(Special.describe().hooks, ["beforeSave"]);
```

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
