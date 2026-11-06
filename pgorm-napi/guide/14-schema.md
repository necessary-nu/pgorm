# Schema generation

A program's tables have to exist before it can use them. pgorm writes the DDL
that creates them in two ways: from the models you declared, and from DDL
builders for everything a model does not describe — views, sequences,
schemas, extensions and changes to an existing table among them. Both only
build statements. You run each one
yourself, with `execute`, usually frozen into a migration when the change is
made, since a statement built from a declaration follows the declaration as
it changes ([Generated DDL in a
migration](22-migrations.md#generated-ddl-in-a-migration)). This chapter is
the JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/13-schema.md).

Schema generation writes; it does not read. pgorm never inspects a database
to compare it with the models.

## Tables from models

Rust's `Schema` turns an entity into the statements that create its table:
the `CREATE TABLE` with its primary key, its unique keys and a foreign key for
each `belongs_to` relation, the `CREATE TYPE` of each enum its columns use,
an index for each `indexed` column, and its comments. `fromModel(model, {
relations })` hands a declared model to the same Rust code, which describes
it as a `TableShape` and writes its statements as it writes an entity's, and
returns an `EntitySchema`:

- `table`, the `CREATE TABLE`, with a foreign key for each `belongsTo`
  relation in `relations`, which must start at the model;
- `enums`, a `CREATE TYPE .. AS ENUM` for each enum its columns use;
- `indexes`, a `CREATE INDEX` for each `indexed` column;
- `comments`, a `COMMENT ON` for the model's `comment` and each column's.

A [registered Rust entity](24-registered-entities.md)'s `fromEntity` returns
the same. Types come before the tables that use them, and a table before the
tables whose foreign keys point at it. This creates the bakery's first two
tables on an empty database:

<!-- example: empty -->
```ts
import assert from "node:assert/strict";
import { Decimal, ForeignKeyViolationError, fromModel } from "@necessary/pgorm";

const cake = fromModel(Cake, { relations: [cakeBakery] });
assert.deepEqual(cake.enums.map((statement) => statement.inspect().sql), [
  `CREATE TYPE "public"."flavor" AS ENUM ('chocolate', 'vanilla', 'lemon', 'carrot')`,
]);
assert.equal(
  cake.table.inspect().sql,
  `CREATE TABLE "cake" ( "id" serial NOT NULL, "name" varchar NOT NULL, "flavor" public.flavor NOT NULL, "price" decimal(10, 2) NOT NULL, "gluten_free" bool NOT NULL, "bakery_id" integer NOT NULL, PRIMARY KEY ("id"), CONSTRAINT "fk-cake-bakery_id" FOREIGN KEY ("bakery_id") REFERENCES "bakery" ("id") )`,
);

for (const statement of [...cake.enums, fromModel(Bakery).table, cake.table]) await pool.execute(statement);

// The foreign key holds: a cake needs its bakery.
await assert.rejects(
  Cake.insert({ name: "Nowhere Cake", flavor: "vanilla", price: new Decimal("1"), glutenFree: false, bakeryId: 99 })
    .execute(pool),
  ForeignKeyViolationError,
);
```

The cake's table is the one Rust writes for the Rust guide's cake entity,
but for the flavor's type: a model's enum column names its type with its
schema, `public.flavor`, where the Rust enum names none. `fromModel` refuses,
with a `ConstructionError` carrying Rust's message, a relation that is no
`belongsTo` or starts at another table, a model read under an alias, and a
column declared `default: true`, whose default the declaration does not
state, since its table has no `DEFAULT` to write.

Rust's generation names what it writes: a composite primary key
`pk-{table}`, a foreign key `fk-{table}-{columns}`, an index
`idx-{table}-{column}`, each shortened to 47 bytes and a hash where it would
pass PostgreSQL's 63. The builders name a constraint only when you do, as the
foreign key above does; otherwise PostgreSQL chooses the name.

## Enum types

Rust writes an entity's enums from its columns, and one enum's type with
`create_enum_from_active_enum`, which refuses an enum stored some other way.
A model's `enums` are its columns' the same way, each written once however
many columns use it, and `createType(name).values(labels)` writes an enum's
type directly: a model's enum column holds both. PostgreSQL has no
`CREATE TYPE IF NOT EXISTS`, so creating a type twice is the server's error;
run each enum's statement once. Adding a value later is `ALTER TYPE .. ADD
VALUE`, which the [types](#types) builder writes.

## Indexes and comments

An index or a comment is a statement apart from the `CREATE TABLE`, and runs
after it. A model's column marked `indexed` gets an index of its own, and a
`comment` on the model or on a column becomes a `COMMENT ON`, as an entity's
do:

<!-- example: empty -->
```ts
import assert from "node:assert/strict";
import { column, fromModel, model } from "@necessary/pgorm";

const Recipe = model("recipe", {
  renameAll: "snake_case",
  comment: "How each cake is made",
  columns: {
    id: column("i32", { primaryKey: true, autoIncrement: true }),
    cakeId: column("i32", { indexed: true }),
    bakeMinutes: column("i32", { comment: "Minutes in the oven" }),
  },
});

const recipe = fromModel(Recipe);
assert.deepEqual(recipe.indexes.map((index) => index.inspect().sql), [
  `CREATE INDEX "idx-recipe-cake_id" ON "recipe" ("cake_id")`,
]);
assert.deepEqual(recipe.comments.map((comment) => comment.inspect().sql), [
  `COMMENT ON TABLE "recipe" IS 'How each cake is made'`,
  `COMMENT ON COLUMN "recipe"."bake_minutes" IS 'Minutes in the oven'`,
]);

for (const statement of [recipe.table, ...recipe.indexes, ...recipe.comments]) await pool.execute(statement);
const comment = await pool.one("SELECT obj_description('recipe'::regclass, 'pg_class') AS comment");
assert.equal(comment.comment, "How each cake is made");
```

`createIndex`, `commentOnTable` and `commentOnColumn` write the same
statements for a table no model describes, and an index can take the name
generation would give it.

An index over several columns, a unique index, a partial index or one of
another method is the [index builder](#indexes)'s too. A unique key over
several columns is part of the `CREATE TABLE`, through its `unique`; a model's
`uniqueKeys` declare the same keys for the model ([Entities](03-entities.md)),
and a [disjoint model](15-disjoint.md)'s schema writes them.

## The DDL builders

The module has a builder for each DDL statement PostgreSQL has a use for
here: tables, indexes, foreign keys, types, domains, sequences, views,
materialized views, schemas, extensions and comments. Each is started by a
function (`createTable`, `createIndex`, `createType`, ...), and every method
returns a new statement, so a base statement can be extended in two
directions.

Rust names each table, column and type with a typed name: an entity, a type
deriving `SqlName`, an `alias` or a `Name::runtime`, so that no string can
stand where a name belongs. JavaScript takes names as strings, and a `Table`
or `TypeName` where a schema qualifies one. Every name is an identifier: the
builders quote each one they write, so `"order"` and a name holding a quote
are written safely, and no name is ever read as SQL. A model's `table` is a
`Table`.

PostgreSQL takes no parameters in DDL, so a value a statement carries, a
`DEFAULT`, a `CHECK`'s operands, an enum label or a comment, is written into
it as an escaped literal, and `inspect()` gives the SQL with no values. Rust
renders such a statement with `to_string()`; JavaScript runs it with
`execute`, which sends that text.

### Tables

`createTable(table)` takes the table's name and then its columns, keys,
foreign keys and checks. A [`ColumnDef`](../lib/schema.d.ts) describes a
column: its name and type, then `notNull`, `default`, `identity`,
`generated` and the rest. A type is a built-in type's name, a `DataType` for
one with a size, such as `new DataType("numeric", { precision: 10, scale: 2
})`, or a `TypeName`:

```ts
import assert from "node:assert/strict";
import { alterTable, col, ColumnDef, createTable, dropTable, renameTable } from "@necessary/pgorm";

const create = createTable("delivery")
  .column(new ColumnDef("id", "integer").notNull().identity("always"))
  .column(new ColumnDef("order_id", "integer").notNull())
  .column(new ColumnDef("address", "text").notNull())
  .column(new ColumnDef("due_at", "timestamptz").notNull())
  .column(new ColumnDef("fragile", "boolean").notNull().default(false))
  .primaryKey("id")
  .foreignKey("order_id", Order.table, "id", { onDelete: "cascade" })
  .check(col("address").ne(""));
assert.equal(
  create.inspect().sql,
  [
    `CREATE TABLE "delivery" (`,
    `"id" integer NOT NULL GENERATED ALWAYS AS IDENTITY,`,
    `"order_id" integer NOT NULL,`,
    `"address" text NOT NULL,`,
    `"due_at" timestamp with time zone NOT NULL,`,
    `"fragile" bool NOT NULL DEFAULT FALSE,`,
    `PRIMARY KEY ("id"),`,
    `FOREIGN KEY ("order_id") REFERENCES "order" ("id") ON DELETE CASCADE,`,
    `CHECK ("address" <> '')`,
    `)`,
  ].join(" "),
);
await pool.execute(create);

// A column added later, and the table renamed.
const alter = alterTable("delivery").addColumn(new ColumnDef("notes", "text"));
assert.equal(alter.inspect().sql, `ALTER TABLE "delivery" ADD COLUMN "notes" text`);
await pool.execute(alter);

const rename = renameTable("delivery", "courier_run");
assert.equal(rename.inspect().sql, `ALTER TABLE "delivery" RENAME TO "courier_run"`);
await pool.execute(rename);

const drop = dropTable("courier_run", { ifExists: true });
assert.equal(drop.inspect().sql, `DROP TABLE IF EXISTS "courier_run"`);
await pool.execute(drop);
```

`alterTable(table)` has no action of its own, and nothing to run until it has
one: its first of `addColumn`, `modifyColumn`, `dropColumn`, `addPrimaryKey`,
`addUnique`, `addForeignKey`, `addCheck`, `addNotNull`, `dropConstraint`,
`validateConstraint`, `alterConstraint`, `setExpression` and `dropExpression`
gives the statement, which takes more. `notValid` adds a foreign key, `CHECK`
or `NOT NULL` without checking the rows already there, which
`validateConstraint` checks later. `renameColumn` and `truncateTable` do what
they say. A generated column is `new ColumnDef(name, type).generated(expr,
"stored")`, or `"virtual"`.

### Foreign keys and their timing

A foreign key is checked at the end of each statement unless it says
otherwise. Its `deferrability` moves the check: `"deferrableInitiallyDeferred"`
checks it once, at commit, which lets a transaction write rows in an order no
single statement could, such as two rows that point at each other. PostgreSQL
18 also takes `enforcement: "notEnforced"`: a key that is recorded, for the
people and tools reading the schema, but never checked. A key added to an
existing table is `alterTable(..).addForeignKey(..)`:

```ts
import assert from "node:assert/strict";
import { alterTable, ColumnDef } from "@necessary/pgorm";

// Each bakery names its head baker, and each baker their bakery.
await pool.execute(alterTable(Bakery.table).addColumn(new ColumnDef("head_baker_id", "integer")));
const headBaker = alterTable(Bakery.table)
  .addForeignKey("head_baker_id", Baker.table, "id", { deferrability: "deferrableInitiallyDeferred" });
assert.equal(
  headBaker.inspect().sql,
  `ALTER TABLE "bakery" ADD FOREIGN KEY ("head_baker_id") REFERENCES "baker" ("id") DEFERRABLE INITIALLY DEFERRED`,
);
await pool.execute(headBaker);

// Point the bakery at a baker who does not exist yet: the key is checked at
// commit, by which time the baker does.
await pool.transaction(async (tx) => {
  await tx.execute(`UPDATE "bakery" SET "head_baker_id" = 4 WHERE "id" = 1`);
  await Baker.insert({ name: "Di", hiredOn: Temporal.PlainDate.from("2026-11-01"), bakeryId: 1 }).execute(tx);
});
const sunrise = await pool.one(`SELECT "head_baker_id" FROM "bakery" WHERE "id" = 1`);
assert.equal(sunrise.head_baker_id, 4);
```

### Indexes

`createIndex(table, entry, { name })` takes the table and its first entry, a
column's name, an expression, or either with an `order` and an
`operatorClass`; `column` adds more. `unique`, `nullsNotDistinct`, `include`
(a covering index's extra columns), `using` (`gin`, `hash`, `gist` or another
method) and `where` (a partial index) say what kind of index it is:

```ts
import assert from "node:assert/strict";
import { col, createIndex, dropIndex } from "@necessary/pgorm";

// One cake of each name per bakery.
const unique = createIndex(Cake.table, "bakery_id", { name: "uq-cake-bakery-name" }).column("name").unique();
assert.equal(unique.inspect().sql, `CREATE UNIQUE INDEX "uq-cake-bakery-name" ON "cake" ("bakery_id", "name")`);
await pool.execute(unique);

// Only the gluten-free cakes, which the shop's filter asks for most.
const partial = createIndex(Cake.table, "price", { name: "idx-cake-gluten-free-price" })
  .where(col("gluten_free").eq(true));
assert.equal(
  partial.inspect().sql,
  `CREATE INDEX "idx-cake-gluten-free-price" ON "cake" ("price") WHERE "gluten_free" = TRUE`,
);
await pool.execute(partial);

const drop = dropIndex(Cake.table, "idx-cake-gluten-free-price");
assert.equal(drop.inspect().sql, `DROP INDEX "idx-cake-gluten-free-price"`);
await pool.execute(drop);
```

`dropIndex` takes the table beside the index's name, since an index lives in
its table's schema and the name alone does not say which.

### Types

`createType(name)` creates an enum (`values`), a composite (`attribute`), or a
range (`asRange`, which [Values and types](13-values.md) uses).
`alterType(name)` has no change until one is chosen: `addValue` adds an enum
value, at its end or before or after another, `renameValue` renames one,
`renameTo` renames the type, and a composite takes `addAttribute`,
`dropAttribute`, `alterAttribute` and `renameAttribute`:

```ts
import assert from "node:assert/strict";
import { alterType, createType } from "@necessary/pgorm";

const boxSize = createType("box_size").values(["small", "large"]);
assert.equal(boxSize.inspect().sql, `CREATE TYPE "box_size" AS ENUM ('small', 'large')`);
await pool.execute(boxSize);

const medium = alterType("box_size").addValue("medium", { before: "large" });
assert.equal(medium.inspect().sql, `ALTER TYPE "box_size" ADD VALUE 'medium' BEFORE 'large'`);
await pool.execute(medium);

const address = createType("address").attribute("street", "text").attribute("city", "text");
assert.equal(address.inspect().sql, `CREATE TYPE "address" AS ("street" text, "city" text)`);
await pool.execute(address);

const labels = await pool.one("SELECT enum_range(NULL::box_size)::text AS labels");
assert.equal(labels.labels, "{small,medium,large}");
```

A value added with `ADD VALUE` cannot be used in the transaction that adds
it, so commit the statement that adds one before any statement uses the
value.

### Domains, sequences and schemas

`createDomain(name, base)` makes a domain, a type with a default and checks,
each `check` a condition over `domainValue()`, PostgreSQL's `VALUE` ([Values
and types](13-values.md) uses one as a column). `createSequence(name)` makes a
counter of its own, beyond the ones an identity column keeps. `createSchema`
makes a schema, named by one string:

```ts
import assert from "node:assert/strict";
import { createDomain, createSchema, createSequence, domainValue, Table } from "@necessary/pgorm";

const archive = createSchema("archive", { ifNotExists: true });
assert.equal(archive.inspect().sql, `CREATE SCHEMA IF NOT EXISTS "archive"`);
await pool.execute(archive);

const receiptNo = createSequence(new Table("receipt_no", { schema: "archive" }))
  .asType("bigint")
  .options({ startWith: 1000 });
assert.equal(receiptNo.inspect().sql, `CREATE SEQUENCE "archive"."receipt_no" AS bigint START WITH 1000`);
await pool.execute(receiptNo);
const next = await pool.one("SELECT nextval('archive.receipt_no') AS next");
assert.equal(next.next, 1000n);

const percent = createDomain("percent", "smallint").check(domainValue().between(0, 100));
assert.equal(percent.inspect().sql, `CREATE DOMAIN "percent" AS smallint CHECK (VALUE BETWEEN 0 AND 100)`);
await pool.execute(percent);
```

`alterDomain(name)` changes a domain one action at a time, as PostgreSQL
takes them, and `alterSequence(name)` a sequence's options or its next value.
Rust has two builders named `Schema`, pgorm's generation from entities and
pgorm-query's `CREATE SCHEMA`; JavaScript's is the one function,
`createSchema`. `renameSchema(name, newName)` renames one, and
`dropSchema(names, { ifExists, behavior })` drops one or several. A schema
that holds anything is dropped only with `behavior: "cascade"`, and refused
(`2BP01`) otherwise:

```ts
import assert from "node:assert/strict";
import { createSchema, DatabaseError, dropSchema, renameSchema } from "@necessary/pgorm";

await pool.execute(createSchema("archive"));
await pool.execute(renameSchema("archive", "attic"));
await pool.execute("CREATE TABLE attic.box (id integer)");

await assert.rejects(pool.execute(dropSchema("attic")), (error: unknown) => {
  return error instanceof DatabaseError && error.sqlstate === "2BP01";
});
const dropped = dropSchema("attic", { behavior: "cascade" });
assert.equal(dropped.inspect().sql, `DROP SCHEMA "attic" CASCADE`);
await pool.execute(dropped);
```

### Views and tables from a query

A view is a named query. `createView(name, select)` takes its name and the
query, built with the [query builder](11-query-builder.md), with its values
written into the definition, which binds nothing; `createMaterializedView`
also stores the query's rows until `refreshMaterializedView` refreshes them,
and `createTableAs` makes an ordinary table from them once. A model query's
`statement` is a `Select`, so a view can be a model's read:

```ts
import assert from "node:assert/strict";
import { call, col, createMaterializedView, createView, refreshMaterializedView, select } from "@necessary/pgorm";

const glutenFree = createView(
  "gluten_free_cake",
  Cake.select("id", "name").where(Cake.col("glutenFree").eq(true)).statement,
).orReplace();
assert.equal(
  glutenFree.inspect().sql,
  `CREATE OR REPLACE VIEW "gluten_free_cake" AS SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."gluten_free" = TRUE`,
);
await pool.execute(glutenFree);

const sold = createMaterializedView(
  "cakes_sold",
  select(col("cake_id"), call("sum", col("quantity"))).from(OrderLine.table).groupBy(col("cake_id")),
).columns(["cake_id", "sold"]);
await pool.execute(sold);
await pool.execute(refreshMaterializedView("cakes_sold"));

const fudge = await pool.one("SELECT sold FROM cakes_sold WHERE cake_id = 1");
assert.equal(fudge.sold, 2n);
const count = await pool.one("SELECT count(*) AS cakes FROM gluten_free_cake");
assert.equal(count.cakes, 3n);
```

A view's options are `orReplace`, `columns`, `checkOption`, `securityBarrier`
and `securityInvoker`; `createRecursiveView` names its columns and takes no
check option. A query PostgreSQL would not take in that place is refused with
a `ConstructionError`, where Rust's constructors return a `Result`.

### Extensions and comments

`createExtension` installs an extension the server has available, such as
`btree_gist`, which a temporal key over a scalar column needs.
`commentOnTable` and `commentOnColumn` write `COMMENT ON` for any table:

```ts
import assert from "node:assert/strict";
import { commentOnColumn, createExtension } from "@necessary/pgorm";

const btreeGist = createExtension("btree_gist", { ifNotExists: true });
assert.equal(btreeGist.inspect().sql, `CREATE EXTENSION IF NOT EXISTS "btree_gist"`);
await pool.execute(btreeGist);

const note = commentOnColumn(Cake.table, "price", "In kroner, VAT included");
assert.equal(note.inspect().sql, `COMMENT ON COLUMN "cake"."price" IS 'In kroner, VAT included'`);
await pool.execute(note);
```

An options object holding a key its builder does not know is a `TypeError`,
so a misspelt option is never left out silently.
