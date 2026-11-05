# PostgreSQL 18 and 19

pgorm targets PostgreSQL 18 or later, and nothing older. A Rust build targets
one major release, 18 unless it asks for 19, and the release decides two
things: which statements the builders can make, and which answers the
documentation and the tests expect where the two releases' servers answer one
statement differently. This chapter is the JavaScript dialect of the [Rust
guide's chapter](../../docs/guide/23-postgres-releases.md) of the same name.

## The default: PostgreSQL 18

The JavaScript package targets PostgreSQL 18, and no build of it reaches back
to 17. It uses what 18 added wherever it fits. Among what this guide covers:

- *virtual generated columns*, computed when read rather than stored
  (`generated(expression, "virtual")` on a `ColumnDef`, and
  `generated: "always"` on a model's column, [Entities](03-entities.md));
- *temporal keys*, a primary or unique key `WITHOUT OVERLAPS` and a foreign
  key over a `PERIOD` (the `withoutOverlaps` and `period` options of the DDL
  builders, [Schema generation](14-schema.md));
- `uuidv7()`, a time-ordered UUID a key can default to;
- `RETURNING` the old and the new version of a written row
  ([Writing rows](04-writing.md));
- named `NOT NULL` constraints, and constraints declared `NOT ENFORCED`
  ([Schema generation](14-schema.md)).

Here is when each of the bakery's ovens is booked, built with the DDL
builders: a temporal key, a default from `uuidv7()` and a virtual column. A
temporal key over a plain column beside the range, such as `oven_id` here,
needs the `btree_gist` extension in the database:

<!-- example: empty -->
```ts
import assert from "node:assert/strict";
import {
  call, col, ColumnDef, createExtension, createTable, ExclusionViolationError, Table,
} from "@necessary/pgorm";

const create = createTable(new Table("oven_booking"))
  .column(new ColumnDef("oven_id", "integer").notNull())
  .column(new ColumnDef("during", "tstzrange").notNull())
  .column(new ColumnDef("batch", "uuid").notNull().default(call("uuidv7")))
  .column(new ColumnDef("trays", "integer").notNull())
  .column(new ColumnDef("cakes", "integer").notNull().generated(col("trays").mul(12), "virtual"))
  .primaryKey("oven_id", { withoutOverlaps: "during" });
assert.equal(
  create.inspect().sql,
  `CREATE TABLE "oven_booking" ( "oven_id" integer NOT NULL, "during" tstzrange NOT NULL, "batch" uuid NOT NULL DEFAULT UUIDV7(), "trays" integer NOT NULL, "cakes" integer NOT NULL GENERATED ALWAYS AS ("trays" * 12) VIRTUAL, PRIMARY KEY ("oven_id", "during" WITHOUT OVERLAPS) )`,
);

await pool.execute(createExtension("btree_gist", { ifNotExists: true }));
await pool.execute(create);

const book = "INSERT INTO oven_booking (oven_id, during, trays) VALUES ($1, tstzrange($2, $3), $4) RETURNING cakes";
const at = (time: string) => Temporal.Instant.from(`2026-11-04T${time}:00Z`);
const booked = await pool.one(book, [1, at("06:00"), at("08:00"), 3]);
assert.equal(booked.cakes, 36);

// Oven 1 is booked from six to eight, so seven to nine overlaps it.
await assert.rejects(pool.one(book, [1, at("07:00"), at("09:00"), 2]), ExclusionViolationError);
```

## Asking for PostgreSQL 19

Rust's `pg-19` cargo feature targets PostgreSQL 19. It only adds: what 19
alone accepts arrives behind it as new items, and nothing an 18 build has
changes shape or meaning, so a statement built the same way renders the same
SQL under either target. pgorm forwards the feature to pgorm-query, and the
Python binding forwards it to pgorm. A Rust program reads which release a
build targets from the constant `pgorm_query::POSTGRES_TARGET`, and a Python
program from `pgorm.capabilities()`.

**Not available in JavaScript yet:** a build targeting PostgreSQL 19. The
`pgorm-napi` crate has no `pg-19` feature to forward to pgorm, so its addon,
and every package built from it, targets 18 alone, with none of what only 19
accepts; and the module reports no target release, having no counterpart of
`POSTGRES_TARGET` or of Python's `capabilities()`. What it does report is the
pgorm release it is built from, `version`:

```ts
import assert from "node:assert/strict";
import { version } from "@necessary/pgorm";

assert.match(version, /^\d+\.\d+\.\d+/);
```

## Where the two releases answer differently

Some statements both releases accept, but answer differently. A builder value
records only what the statement says, not what the server holds, so pgorm
renders the statement the same way for both. Three differences exist today:

- `ALTER CONSTRAINT .. ENFORCED` and `NOT ENFORCED` reach a `CHECK` constraint
  from 19, and only a foreign key on 18;
- `SET EXPRESSION` on a virtual generated column that a `CHECK` reads is
  refused on 18 and accepted on 19;
- `JSON_ARRAY` over a query of no rows is `NULL` on 18 and the empty array on
  19.

Code that runs against both reads the last as `null` or an array. Against the
PostgreSQL 18 server the guide's tests run on, it is `null`:

```ts
import assert from "node:assert/strict";
import { jsonArrayQuery, select, Table } from "@necessary/pgorm";

// The names of the cakes over 10.00: there are none.
const cake = new Table("cake");
const dear = select(cake.col("name")).from(cake).where(cake.col("price").gt(10));
const { names } = await pool.one(select(jsonArrayQuery(dear).as("names")));

const { release } = await pool.one("SELECT current_setting('server_version_num')::int / 10000 AS release");
assert.deepEqual(names, release === 18 ? null : []);
```

## The grammar stays at 18 for now

Every SQL parse pgorm makes uses PostgreSQL 18's grammar, whichever release a
build targets, because the parser, libpg_query, has no 19 grammar yet. The
statements JavaScript's builders render are pgorm-query's, and everything a
builder renders has to parse, so no builder makes syntax only 19 parses.
