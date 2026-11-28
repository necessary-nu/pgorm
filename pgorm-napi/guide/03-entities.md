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
import { col, column, DataType, dec, fromModel, model } from "@necessary/pgorm";

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
const line = await OrderLine.insert({ orderId: 1, cakeId: 2, price: dec`3.20` })
  .returning("quantity", "total")
  .one(pool);
assert.equal(line.quantity, 1);
assert.equal(line.total.toString(), "3.20");
```

Rust's entities can also mark a column `immutable`, written by its row's
insert and refused in every update and upsert after, which is Rust-only for
now: a declared column has no such option.

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
import { ConstructionError, dec } from "@necessary/pgorm";

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
      price: dec`3.00`,
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
  by its kind, `"i16"` or `"text"`, and its `decode` and `encode` map the
  stored values to the program's and back ([Values and
  types](13-values.md#json)); Rust's `display` is the program's own
  spelling.

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
| `Entity`, on a hand-written `Entity` | `model(..)` takes names computed at run time, [below](#names-computed-at-run-time) |
| `SqlName`, `StaticName` | a name is a string, quoted wherever it is written ([The query builder](11-query-builder.md)) |

A Rust `ValueType` newtype keeps, say, grams of flour apart from other
integers at compile time, and a `JsonValueType` reads a JSON document as a
struct, its shape checked as it is read. A column's `decode` and `encode`
are the JavaScript form of both: `decode` turns each value a record reads
into the program's own, a throw making the read a `DecodeError`, and the
record's field is typed as what it returns ([Values and
types](13-values.md#json)).

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
hand-written `Entity` derives `pgorm::Entity` and computes them. A JavaScript
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

## Scopes

Some tables are read only under a condition the program supplies: a
company's notes by its members, a user's notes by that user. Forgetting the
condition once lets a statement read another company's rows. A *scope*
holds a table's statements to it: every statement on a scoped model is
built from a *context* holding its scopes, and every way to build one
without such a context is refused as the statement is built, before
anything is sent, in the words the [Rust guide's
Scopes](../../docs/guide/02-entities.md#scopes) section shows Rust's compiler
using. pgorm writes the condition; holding the rows to it in the database,
with row-level security or otherwise, stays the program's.

Here JavaScript differs from Rust: JavaScript has no compile time, so where
Rust's compiler stops a statement, the builder throws a `ConstructionError`
as the statement is built. TypeScript catches most of it before then: a
scoped model's forms taking no context, a context missing one of its scopes,
and a plain join into a scoped model are type errors.

### Declaring scopes

`scope("CompanyId")` declares a scope, named as Rust names one by its type.
A column holds a scope where its options say so with `scope`, and the
column's kind converts the scope's values as it converts a value written to
it. A column's kind alone never makes it a scope column:

```ts
import assert from "node:assert/strict";
import { column, model, scope } from "@necessary/pgorm";

const CompanyId = scope("CompanyId");
const UserId = scope("UserId");

const Note = model("note", {
  columns: {
    id: column("i64", { primaryKey: true, generated: "byDefault" }),
    companyId: column("i64", { name: "company_id", scope: CompanyId }),
    authorId: column("i64", { name: "author_id", scope: UserId }),
    body: column("text"),
  },
});

assert.deepEqual(Note.scopes, ["CompanyId", "UserId"]);
// A scope column is never NULL: a row is in one value of its scope.
assert.throws(() => column("i64", { scope: UserId, nullable: true }), /never NULL/);
```

A model holds each scope in one column. A scope column is written when its
row is inserted and never changed after, since a row cannot leave its
scope; a nullable, array or generated one is refused where it is declared.
A primary key can be a scope column too: a company's `id` is a `CompanyId`,
so a member reads their own company's row and no other.

### Contexts

A *context* is what a statement on a scoped model is built under. Calling a
scope makes a value of it, `CompanyId(1n)`, and a value is a context holding
itself; a plain object of values is a context holding each of them.
`every(UserId)` holds every value of `UserId`: the context lifts that scope
on purpose, and the statement gets no condition for it. A member is signed
in to one company as one user, and a company's administrator sees every one
of its users:

```ts
import assert from "node:assert/strict";
import { column, every, model, scope } from "@necessary/pgorm";

const CompanyId = scope("CompanyId");
const UserId = scope("UserId");

const Company = model("company", {
  columns: { id: column("i64", { primaryKey: true, scope: CompanyId }), name: column("text") },
});
const Note = model("note", {
  columns: {
    id: column("i64", { primaryKey: true, generated: "byDefault" }),
    companyId: column("i64", { name: "company_id", scope: CompanyId }),
    authorId: column("i64", { name: "author_id", scope: UserId }),
    body: column("text"),
  },
});

const ada = { company: CompanyId(1n), user: UserId(10n) };
const admin = { company: CompanyId(1n), users: every(UserId) };
assert.equal(
  Note.findFor(ada).inspect().sql,
  `SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body" FROM "note" WHERE "note"."company_id" = $1 AND "note"."author_id" = $2`,
);
assert.equal(
  Note.findFor(admin).inspect().sql,
  `SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body" FROM "note" WHERE "note"."company_id" = $1`,
);
// A scope's value alone is a context, and the company's key is its scope.
assert.equal(
  Company.findFor(CompanyId(1n)).inspect().sql,
  `SELECT "company"."id", "company"."name" FROM "company" WHERE "company"."id" = $1`,
);
```

A context holds each scope once, as one value or as every value; an object
holding one twice is refused when a statement is built under it. A context
is a scope's value, an `every(..)` or an object of them, and anything else is
a `TypeError`.

### Every statement takes its context first

Each statement constructor has a form ending in `For` that takes the
context first: a model's `selectFor`, `findFor`, `findByKeyFor`, `graphFor`,
`insertFor`, `insertManyFor`, `updateFor` and `deleteFor`; an insert's
`addFor` and `addManyFor`, and a try-insert's; an active model's
`insertFor`, `updateFor` and `deleteFor`; a query's `joinFor` and
`joinRevFor`; a relation's and a related path's `findFor`; a linked chain's
`findFor` and `findLinkedFor`; and `pipeline.fromFor`. An unscoped model
takes any context, and its forms taking none are the ones it always had.

On a scoped model, the forms taking no context are refused, and so is a
`For` form given a context missing one of its scopes:

```ts
import assert from "node:assert/strict";
import { column, model, scope } from "@necessary/pgorm";

const CompanyId = scope("CompanyId");
const UserId = scope("UserId");

const Note = model("note", {
  columns: {
    id: column("i64", { primaryKey: true, generated: "byDefault" }),
    companyId: column("i64", { name: "company_id", scope: CompanyId }),
    authorId: column("i64", { name: "author_id", scope: UserId }),
    body: column("text"),
  },
});

// @ts-expect-error: a scoped model's find() takes a context: findFor(cx)
assert.throws(() => Note.find(), {
  message:
    "`note` is scoped by `CompanyId` and `UserId`, which a statement taking no context does not hold: a statement on `note` starts from a `For` form, given a context holding its scopes first: `findFor(cx)`, `insertFor(cx, values)`, `updateFor(cx, values)`",
});
// @ts-expect-error: the context holds no UserId
assert.throws(() => Note.findFor(CompanyId(1n)), {
  message: "`note` is scoped by `CompanyId` and `UserId`, which `CompanyId` does not hold",
});
```

The refusal names the table, its scopes and the context, as Rust's compile
error names the entity and the context's type. Each `@ts-expect-error` is
TypeScript's own refusal of the same call: a model's columns carry the scopes
they hold, so `find` is typed for a model holding none, and `findFor` for a
context holding every scope the model's columns name. A loader is given a
query rather than a model: hand a relation's `load` a `findFor` query, since
the model alone is every row and is refused for a scoped one.

### Reading and joining

A read ANDs each scope its context holds the value of into the `WHERE`, so
Ada reads her own notes and the administrator reads the company's. A graph
keeps its context, and each slot's scopes go in its join's `ON`; a query's
`joinFor` puts the joined table's there too, and a plain `join` into a
scoped table is refused:

```ts
import assert from "node:assert/strict";
import { column, DecodeError, every, fromModel, model, scope } from "@necessary/pgorm";

const CompanyId = scope("CompanyId");
const UserId = scope("UserId");

const Company = model("company", {
  columns: { id: column("i64", { primaryKey: true, scope: CompanyId }), name: column("text") },
});
const Note = model("note", {
  columns: {
    id: column("i64", { primaryKey: true, generated: "byDefault" }),
    companyId: column("i64", { name: "company_id", scope: CompanyId }),
    authorId: column("i64", { name: "author_id", scope: UserId }),
    body: column("text"),
  },
});
const companyNotes = Company.hasMany(Note, { from: "id", to: "companyId" });

for (const declared of [Company, Note]) await pool.execute(fromModel(declared).table);
for (const [id, name] of [[1n, "Acme"], [2n, "Globex"]] as const) {
  await Company.insertFor(CompanyId(id), { name }).execute(pool);
}
for (const [company, user, body] of [[1n, 10n, "Ada's plan"], [1n, 11n, "Bo's draft"], [2n, 20n, "Cy's memo"]] as const) {
  await Note.insertFor({ company: CompanyId(company), user: UserId(user) }, { body }).execute(pool);
}

const ada = { company: CompanyId(1n), user: UserId(10n) };
const admin = { company: CompanyId(1n), users: every(UserId) };
const hers = await Note.findFor(ada).all(pool);
assert.deepEqual(hers.map((note) => note.body), ["Ada's plan"]);
const theirs = await Note.findFor(admin).orderBy(Note.col("id").asc()).all(pool);
assert.equal(theirs.length, 2, "Ada's and Bo's notes, and not Globex's");

// Cy's note belongs to Globex, so Ada finds no note of its key.
await assert.rejects(Note.findByKeyFor(ada, { id: 3n }).one(pool), DecodeError);

// A graph keeps its context, and each slot's scopes go in its join's ON.
const acme = await Company.graphFor(ada).joinMaybe(companyNotes).all(pool);
assert.deepEqual(acme.map(([company, note]) => [company.name, note?.authorId]), [["Acme", 10n]]);

assert.equal((await Company.findFor(ada).joinFor(ada, companyNotes).all(pool)).length, 1);
// @ts-expect-error: a scoped model joins by joinFor
assert.throws(() => Company.findFor(ada).join(companyNotes), /join it with a form taking the statement's context, such as `joinFor`/);
```

A pipeline reads a scoped model through `pipeline.fromFor`, which filters
its rows by each scope value the context holds, binding the value; the model
given to `pipeline.from` or as a join's source is refused, and another scoped
model joins the pipeline as a pipeline of its own, `pipeline.fromFor(cx,
other)`.

### Writing

An insert under a context fills each scope column the row leaves out with
the value the context holds, and refuses a row setting one to another value.
Under `every` of a scope the insert fills nothing: the row sets the column
itself, and a row that does not is refused.

An update or a delete under a context carries the scopes in its `WHERE`,
before any condition of its own, so a row outside them matches nothing: an
active model's `updateFor` of such a key is a `DecodeError`, as Rust's
`RecordNotFound` is, and a delete counts `0`. A scope column is written when
its row is inserted and never changed after: an update setting one, and an
upsert whose `ON CONFLICT .. DO UPDATE` names one, are refused before
anything is sent.

```ts
import assert from "node:assert/strict";
import { column, DecodeError, every, fromModel, model, scope } from "@necessary/pgorm";

const CompanyId = scope("CompanyId");
const UserId = scope("UserId");

const Note = model("note", {
  columns: {
    id: column("i64", { primaryKey: true, generated: "byDefault" }),
    companyId: column("i64", { name: "company_id", scope: CompanyId }),
    authorId: column("i64", { name: "author_id", scope: UserId }),
    body: column("text"),
  },
});

await pool.execute(fromModel(Note).table);
const ada = { company: CompanyId(1n), user: UserId(10n) };
const bo = { company: CompanyId(1n), user: UserId(11n) };
const admin = { company: CompanyId(1n), users: every(UserId) };
await Note.insertFor(bo, { body: "Bo's draft" }).execute(pool);

const written = await Note.insertFor(ada, { body: "Ada's plan" }).returning().one(pool);
assert.deepEqual([written.companyId, written.authorId], [1n, 10n]);
assert.throws(
  () => Note.insertFor(ada, { companyId: 2n, body: "for Globex" }),
  /the row sets `note`.`company_id` to a value outside the context's `CompanyId`/,
);
assert.throws(
  () => Note.insertFor(admin, { body: "whose?" }),
  /the context holds every `UserId`, so the row sets `note`.`author_id` itself/,
);

// Bo's note is the company's, so the administrator edits it and Ada cannot.
const edit = Note.active({ id: 1n, body: "Bo's draft, reviewed" });
await assert.rejects(edit.updateFor(ada, pool), DecodeError);
assert.equal((await edit.updateFor(admin, pool)).body, "Bo's draft, reviewed");

// Handing Bo's note to Ada would move it out of Bo's scope.
assert.throws(
  // @ts-expect-error: a scope field is never updated
  () => Note.updateFor(admin, { authorId: 10n }),
  /`note`.`author_id` is written when its row is inserted and never changed/,
);
assert.equal(await Note.deleteFor(ada).where(Note.col("id").eq(1n)).execute(pool), 0);
```

A scope written by hand, Rust's `ScopeOn` with a condition of its own such
as a message's sender or recipient, is Rust's alone: a declared model's
scopes are its scope columns. A declared disjoint model or one-of reference
reads its models under no context, so one over a scoped model is refused as
it is declared.

### Registered entities

A Rust entity that declares scopes stays held to them from JavaScript. The
application crate registers each scope type before the entity, and the
entity by `scoped` rather than `entity`:

```rust
registry.scope::<notes::CompanyId>()?;
registry.scope::<notes::UserId>()?;
registry.scoped::<notes::note::Entity>("app.Note")?;
```

A context names a registered scope by its Rust type's name, so
`scope("CompanyId")` is the Rust `CompanyId`, its value converted by its
`ValueType`. Every statement on the registration is the Rust entity's own
`_for` form under that context: `findFor`, `insertFor`, `insertManyFor`,
`updateFor`, `updateManyFor`, an active model's `insertFor`, `updateFor` and
`deleteFor`, a query's `joinFor` and `pipeline.fromFor`, its written scopes
included. The forms taking no context are refused as they are built, as a
declared model's are, and a registration's `scopes` lists the scopes its
entity declares. A graph, a source tuple, a disjoint enum and a one-of
reference register over unscoped entities alone. [Registered Rust
entities](24-registered-entities.md) says how to build such a module.

### Raw SQL

SQL text with its parameters, `findByStatement` and the statement builders
used directly are outside scopes: pgorm does not read the SQL they hold. A
read meant to cross scopes, such as a report over every company, is written
with them.
