# PostgreSQL 19

pgorm targets PostgreSQL 19, and nothing older. No feature names a release:
every addon makes the statements 19 accepts, the documentation gives 19's
answers, and pgorm's own tests run against a 19 server. An application whose
server runs an older release moves it to 19 before it moves to this pgorm.
This chapter is the JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/23-postgres-releases.md) of the same name.

The module reports the pgorm release it is built from as `version`:

```ts
import assert from "node:assert/strict";
import { version } from "@necessary/pgorm";

assert.match(version, /^\d+\.\d+\.\d+/);
```

## What pgorm uses

pgorm uses what recent releases added wherever it fits. Among what this guide
covers, from PostgreSQL 18:

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

## Coming from PostgreSQL 18

Some statements 18 and 19 both accept, but answer differently. A builder value
records only what the statement says, not what the server holds, so the
statement renders as it did, and the server's answer is 19's:

- `ALTER CONSTRAINT .. ENFORCED` and `NOT ENFORCED` reach a `CHECK` constraint
  as they reach a foreign key, `ENFORCED` checking the rows already there;
- `SET EXPRESSION` on a virtual generated column that a `CHECK` reads checks
  the rows against the `CHECK`, where 18 refused it;
- `JSON_ARRAY` over a query of no rows is the empty array, where 18 answered
  `null`.

```ts
import assert from "node:assert/strict";
import { jsonArrayQuery, select, Table } from "@necessary/pgorm";

// The names of the cakes over 10.00: there are none.
const cake = new Table("cake");
const dear = select(cake.col("name")).from(cake).where(cake.col("price").gt(10));
const { names } = await pool.one(select(jsonArrayQuery(dear).as("names")));

assert.deepEqual(names, []);
```

## The grammar is PostgreSQL 19's

Every SQL parse pgorm makes uses one parser, pgsql-syntax, a Rust port of
PostgreSQL 19's own scanner and parser. `sql()` and `prql()`
([Raw SQL](10-raw-sql.md)) check text with it, through the code Rust's `sql!`
and `prql!` run; `modelsFromSql` ([Generating entities](23-codegen.md)) reads
DDL with it; and the pipeline's reading of the SQL it compiled, the
paginator's reading of raw SQL and query fingerprints in
[Metrics](21-metrics.md) use it, as an application addon's `sql!` and `prql!`
do. A check of text is a check against 19's grammar, so what 19 added passes
it:

```ts
import assert from "node:assert/strict";
import { sql } from "@necessary/pgorm";

// REPACK and ON CONFLICT .. DO SELECT are PostgreSQL 19's.
assert.equal(sql("REPACK cake"), "REPACK cake");
const upsert = "INSERT INTO cake (id) VALUES (1) ON CONFLICT (id) DO SELECT RETURNING *";
assert.equal(sql(upsert), upsert);
```

19's grammar also refuses a little that 18's parsed. It makes `ignore` and
`respect` keywords that a column label needs `AS` before, so a bare
`SELECT 1 ignore`, which an 18 server ran, is refused; `SELECT 1 AS ignore`
parses. pgorm writes `AS` before every column label it renders, so no
statement a builder makes is affected. The five keywords 19 adds, `ignore`,
`lsn`, `repack`, `respect` and `wait`, are all unreserved, so pgorm quotes the
identifiers it quoted before.
