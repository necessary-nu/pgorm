# The pipeline

A pipeline is a query written as a list of steps. Each step takes a relation
and returns a relation: keep these rows, add this column, group by that key,
sort, take the first ten. The module's `pipeline` namespace compiles the steps
to one PostgreSQL statement, in the shape of a [PRQL] query, through the same
compiler pgorm's Rust pipeline uses. This chapter is the JavaScript dialect of
the [Rust guide's chapter](../../docs/guide/11-pipeline.md).

Where each step lands in the SQL follows from where it stands in the list. A
filter before an aggregation becomes `WHERE`, a filter after one becomes
`HAVING`, and a filter after a window function wraps the steps before it in a
`WITH` query, because SQL cannot filter on a window in the same `SELECT`. You
write the steps in the order you think of them, and the compiler writes the
clauses.

Reach for a pipeline for reports and analysis: aggregates you filter on,
rankings, running totals, a relation built from other relations. A model's
`find()` reads its rows into records; the [query builder](11-query-builder.md)
writes SQL clause by clause; a pipeline sits between them, composing
relations rather than clauses.

[PRQL]: https://prql-lang.org

## A first pipeline

`pipeline.from(source)` starts a pipeline, and each method appends one step,
returning a new pipeline. `inspect()` compiles it, returning the SQL and the
values it binds:

```ts
import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";

const cake = (column: string) => pl.col("cake", column);

const { sql, values } = pl.from(Cake.table)
  .filter(cake("gluten_free").eq(pl.literal(true)))
  .sort(cake("price").desc())
  .take(2)
  .select(cake("name"), cake("price"))
  .inspect();

assert.equal(
  sql,
  "SELECT name, price FROM cake WHERE gluten_free = true ORDER BY price DESC LIMIT 2",
);
assert.deepEqual(values, []);
```

The pipeline reads a relation, not a model. A source is a `Table`, read under
its alias if it has one; a table's name; another pipeline; a named source,
[below](#a-table-meeting-itself); or a [registered Rust
entity](24-registered-entities.md). A declared model is none of those, so a
pipeline reads its `table`, and a model given whole is refused with a
`TypeError`. Its columns are written with `pipeline.col(table, column)`, by
their SQL names: the compiler has no catalog, so every column names its
relation, and the SQL leaves the qualifier out where only one relation is
read. Rust's pipeline starts from an entity and names its columns by the
entity's `Column` variants, its comparison operators coming from the
`ExprOps` trait; a JavaScript pipeline's expressions are the namespace's own,
and do not mix with the statement builders' `Expr` or a model's columns.

**Not available in JavaScript yet:** reading a declared model in a pipeline.
Rust's `Pipeline::from` takes an entity, and its steps take the entity's
columns, each compared as its column type says. JavaScript's pipeline takes a
model's `table` and its columns' SQL names, and a column whose type needs a
cast to be compared needs the cast written out, with `comparesAs`, as
[Values from the program](#values-from-the-program) shows.

`pipeline.literal(value)` writes a constant into the SQL. That is the other
way round from Rust, where a literal `true` is written into the SQL and a
value is bound only through a binder: in JavaScript any plain value an
operand is given is bound as a parameter, and `literal` is how a constant
reaches the SQL text.

A pipeline runs through its own terminals. `all`, `one` and `optional` run it
on a pool, a connection or a transaction, as Rust's `all`, `one` and
`one_opt` do, and `one` and `optional` append `take 1` first. Each row is a
plain object keyed by the names the SQL gives its columns:

```ts
import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";

const cake = (column: string) => pl.col("cake", column);

const dearest = await pl.from(Cake.table)
  .filter(cake("gluten_free").eq(pl.literal(true)))
  .sort(cake("price").desc())
  .take(2)
  .select(cake("name"), cake("price"))
  .all(pool);

assert.deepEqual(
  dearest.map((row) => [row.name, String(row.price)]),
  [["Chocolate Torte", "6.00"], ["Lemon Drizzle", "3.20"]],
);
```

Rust decodes a pipeline's rows through `into_tuple` or `into_model`, by
position or into a type of your choosing. JavaScript's rows are the plain
rows every query gives, each value decoded by its column's type ([Values and
types](13-values.md)), so a `numeric` reads as a `Decimal` and an enum as its
label. A pipeline also runs through a pool's `query`, `one`, `optional` and
`stream`, as any statement does, with their cardinality: there `one` wants
exactly one row.

## The steps

| Step | What it does | SQL it usually becomes |
| --- | --- | --- |
| `from` | the relation to start from | `FROM` |
| `filter` | keep the rows a condition holds for | `WHERE`, or `HAVING` after an aggregation |
| `derive` | add computed columns, keeping the others | a projection |
| `select` | replace the columns with these | a projection |
| `group` and `aggregate` | one row per key, with aggregates | `GROUP BY` |
| `window` | add columns computed over a window | `OVER (..)` |
| `sort` | order the rows | `ORDER BY` |
| `take`, `takeRange` | keep the first rows, or a 1-based range of them | `LIMIT`, `OFFSET` |
| `join` | combine with another relation on a condition | `JOIN` |
| `append`, `intersect`, `remove` | combine with another relation's rows | `UNION ALL`, `INTERSECT ALL`, `EXCEPT ALL` |
| `distinct` | keep one copy of each row | `DISTINCT` |

A step that takes several expressions takes them as arguments:
`.select(cake("name"), total)`. Rust passes one bare, a homogeneous list as an
array, or a mixed list as a tuple.

## Names

A column a step introduces needs a name the later steps can refer to.
`pipeline.alias(name)` is that name: it is made once, attached with `as`, and
used from then on as an expression like any column. Grouping the cakes by
bakery and keeping the bakeries with more than two:

```ts
import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";

const cake = (column: string) => pl.col("cake", column);
const cakes = pl.alias("cakes");
const mean = pl.alias("mean_price");

const busy = pl.from(Cake.table)
  .group(cake("bakery_id"))
  .aggregate(pl.countRows().as(cakes), pl.average(cake("price")).as(mean))
  .filter(cakes.gt(pl.literal(2)));

assert.equal(
  busy.inspect().sql,
  "SELECT bakery_id, COUNT(*) AS cakes, AVG(price) AS mean_price FROM cake " +
    "GROUP BY bakery_id HAVING COUNT(*) > 2",
);

const rows = await busy.all(pool);
assert.equal(rows.length, 1);
assert.deepEqual([rows[0]!.bakery_id, rows[0]!.cakes], [1, 3n]);
assert.ok(String(rows[0]!.mean_price).startsWith("4.5666"));
```

The aggregates are `countRows` (`COUNT(*)`), `count` (`COUNT(expr)`, the rows
it is not null on), `countDistinct`, `sum`, `min`, `max`, `average` and
`stddev`. `caseWhen` writes a `CASE` with a mandatory fallback,
`literal(null)` is the `NULL` literal, and `cast` converts to one of the cast
types the declarations list. A count is an `int8`, which reads as a `bigint`.

`derive` adds a column and keeps the rest. Each cake's tier and its flavor as
text, for the second and third cakes by id:

```ts
import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";

const cake = (column: string) => pl.col("cake", column);
const tier = pl.alias("tier");
const flavorName = pl.alias("flavor_name");

const tiers = pl.from(Cake.table)
  .derive(
    pl.caseWhen([[cake("price").gte(pl.literal(4)), pl.literal("premium")]], pl.literal("standard")).as(tier),
    cake("flavor").cast("text").as(flavorName),
  )
  .sort(cake("id"))
  .takeRange(2, 3)
  .select(cake("name"), tier, flavorName);

assert.equal(
  tiers.inspect().sql,
  "WITH table_0 AS (SELECT name, CASE WHEN price >= 4 THEN 'premium' ELSE 'standard' END " +
    "AS tier, CAST(flavor AS text) AS flavor_name, id FROM cake ORDER BY id LIMIT 2 OFFSET 1) " +
    "SELECT name, tier, flavor_name FROM table_0 ORDER BY id",
);

assert.deepEqual(await tiers.all(pool), [
  { name: "Lemon Drizzle", tier: "standard", flavor_name: "lemon" },
  { name: "Carrot Cake", tier: "standard", flavor_name: "carrot" },
]);
```

An alias that collides with a name PRQL itself defines, such as `sum`,
`first` or `from`, is refused when the pipeline compiles, because it would
hide the built-in from every later step. Building the pipeline succeeds; its
`inspect()` and its terminals throw a `ConstructionError`, where Rust's
`into_sql` returns `PipelineError::ReservedAlias`:

```ts
import assert from "node:assert/strict";
import { ConstructionError, pipeline as pl } from "@necessary/pgorm";

const refused = pl.from(Cake.table)
  .group(pl.col("cake", "bakery_id"))
  .aggregate(pl.countRows().as(pl.alias("sum")));

assert.throws(() => refused.inspect(), ConstructionError);
await assert.rejects(refused.all(pool), /alias `sum` collides with a PRQL built-in name/);
```

## Values from the program

A value from the program, such as a price a customer typed, is bound as a
parameter: the statement's text then stays the same from one value to the
next, so a connection prepares it once, and any value pgorm can bind can be
compared, a `Decimal` or a date among them. In JavaScript a plain value is
bound by the step that takes it. Each step also has a `With` form, as Rust's
`_with` forms, whose function is called once with a `Binder`; its `bind`
records the value and returns its `$N` placeholder, which the expressions the
function returns can use more than once:

```ts
import assert from "node:assert/strict";
import { Decimal, pipeline as pl } from "@necessary/pgorm";

const cake = (column: string) => pl.col("cake", column);
const budget = new Decimal("3.50");

const affordable = pl.from(Cake.table)
  .filterWith((binder) => cake("price").lte(binder.bind(budget)))
  .sort(cake("price"))
  .select(cake("name"));

const { sql, values } = affordable.inspect();
assert.equal(
  sql,
  "WITH table_0 AS (SELECT name, price FROM cake WHERE price <= $1) " +
    "SELECT name FROM table_0 ORDER BY price",
);
assert.deepEqual(values.map((value) => String(value.value)), ["3.50"]);

assert.deepEqual(await affordable.all(pool), [{ name: "Vanilla Sponge" }, { name: "Lemon Drizzle" }]);
```

A bound placeholder belongs to the step that minted it. Rust's binder carries
a lifetime, so an expression holding one of its placeholders cannot leave the
closure, and a second pipeline that uses it does not compile. JavaScript has
no lifetimes: the placeholder can escape, and is refused when it is used. Once
its binder's function has returned, any use of it outside the expressions
that function returned is a `LifecycleError`:

```ts
import assert from "node:assert/strict";
import { Decimal, LifecycleError, pipeline as pl } from "@necessary/pgorm";

const price = pl.col("cake", "price");
let leaked: pl.PipelineExpr | undefined;
pl.from(Cake.table).filterWith((binder) => {
  leaked = binder.bind(new Decimal("3.50"));
  return price.lte(leaked);
});

assert.throws(() => pl.from(Cake.table).filter(price.gt(leaked!)), LifecycleError);
```

A `take` takes a number, not an expression, since PRQL has no bound row
count; anything else is a `TypeError`.

A column whose type needs a cast to be compared names that type with
`comparesAs`: the column and each value beside it are then cast to it. A
column over a domain whose base is an enum needs it, since PostgreSQL resolves
no comparison for the domain, and so does a `money` column compared with a
`Decimal`. Rust's pipeline takes the cast from the entity's column type; a
JavaScript pipeline over a table has no column type to take it from:

```ts
import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";

const chocolate = pl.from(Cake.table)
  .filter(pl.col("cake", "flavor").comparesAs(flavor).eq("chocolate"))
  .sort(pl.col("cake", "id"))
  .select(pl.col("cake", "name"));

assert.equal(
  chocolate.inspect().sql,
  "WITH table_0 AS (SELECT name, id FROM cake WHERE CAST(flavor AS public.flavor) = " +
    "CAST($1::text AS public.flavor)) SELECT name FROM table_0 ORDER BY id",
);
assert.deepEqual(await chocolate.all(pool), [{ name: "Chocolate Fudge" }, { name: "Chocolate Torte" }]);
```

## Joins

`join` takes the relation to join, the condition, and which rows to keep as
its `kind`: `"inner"`, the default, `"left"`, `"right"` or `"full"`. Rust
passes a `JoinSide` first. A column names its relation, so a condition across
two tables is qualified as written:

```ts
import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";

const cake = (column: string) => pl.col("cake", column);
const bakery = (column: string) => pl.col("bakery", column);
const cakeName = pl.alias("cake");

const inBergen = pl.from(Cake.table)
  .join(Bakery.table, cake("bakery_id").eq(bakery("id")), { kind: "inner" })
  .filter(bakery("city").eq(pl.literal("Bergen")))
  .select(cake("name").as(cakeName), bakery("name"))
  .sort(cakeName);

assert.equal(
  inBergen.inspect().sql,
  "SELECT cake.name AS cake, bakery.name FROM cake INNER JOIN bakery " +
    "ON cake.bakery_id = bakery.id WHERE bakery.city = 'Bergen' ORDER BY cake",
);

assert.deepEqual(await inBergen.all(pool), [
  { cake: "Carrot Cake", name: "Moonlight" },
  { cake: "Vanilla Sponge", name: "Moonlight" },
]);
```

Rust ends a pipeline with `select_sources` to decode the relations it read as
the entities themselves, each an `Option` of its model, because a right or
full join can leave either side absent. In JavaScript, `selectSources` does
that for a tuple of Rust entities an application
[registered](24-registered-entities.md), its rows decoded into the entities'
records.

**Not available in JavaScript yet:** decoding a pipeline's rows into declared
models' records. Rust's `select_sources` projects each listed entity's columns
and decodes each as its model. JavaScript's `selectSources` takes only a
source tuple an application registered from Rust, so a pipeline over declared
models gives plain rows keyed by column name, as above.

### A table meeting itself

`pipeline.source(relation).named(name)` reads a relation under a name of your
own, which is how a table is joined to itself. The name is then the relation's
only name, and `pipeline.col(name, column)` writes its columns. The pairs of
cakes sold by the same bakery:

```ts
import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";

const cake = (column: string) => pl.col("cake", column);
const other = (column: string) => pl.col("other", column);

const pairs = pl.from(Cake.table)
  .join(pl.source(Cake.table).named("other"), cake("bakery_id").eq(other("bakery_id")))
  .filter(cake("id").lt(other("id")))
  .sort(cake("id"), other("id"))
  .select(cake("name").as(pl.alias("earlier")), other("name").as(pl.alias("later")));

assert.deepEqual(await pairs.all(pool), [
  { earlier: "Chocolate Fudge", later: "Lemon Drizzle" },
  { earlier: "Chocolate Fudge", later: "Chocolate Torte" },
  { earlier: "Lemon Drizzle", later: "Chocolate Torte" },
  { earlier: "Carrot Cake", later: "Vanilla Sponge" },
]);
```

A table no model describes is read the same way, by its name or a `Table`:
`pl.from(new Table("cake", { schema: "archive" }))` reads `archive.cake`,
where Rust writes `Pipeline::from_schema`.

## Windows

`window(over, ...columns)` adds columns computed over a window of rows. The
window comes first, where Rust passes it second: `pipeline.over()` is the
whole relation, `by` partitions it and `sortBy` orders it. The window
functions are `rowNumber`, `rank`, `rankDense`, `lag`, `lead`, `first` and
`last`, and any aggregate. The dearest cake at each bakery:

```ts
import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";

const cake = (column: string) => pl.col("cake", column);
const place = pl.alias("place");

const dearest = pl.from(Cake.table)
  .window(pl.over().by(cake("bakery_id")).sortBy(cake("price").desc()), pl.rowNumber().as(place))
  .filter(place.eq(pl.literal(1)))
  .sort(cake("bakery_id"))
  .select(cake("bakery_id"), cake("name"));

assert.equal(
  dearest.inspect().sql,
  "WITH table_0 AS (SELECT bakery_id, name, ROW_NUMBER() OVER (PARTITION BY bakery_id " +
    "ORDER BY price DESC) AS _expr_0 FROM cake) SELECT bakery_id, name FROM table_0 " +
    "WHERE _expr_0 = 1 ORDER BY bakery_id",
);

assert.deepEqual(await dearest.all(pool), [
  { bakery_id: 1, name: "Chocolate Torte" },
  { bakery_id: 2, name: "Carrot Cake" },
]);
```

A window can be narrowed to a frame: `rows(start, end)` counts rows from the
current one, and `range(start, end)` counts values. Each bound is an offset
from the current row, negative before it and positive after, and `null` leaves
that side unbounded, as Rust's `None` does. A running total of a bakery's
prices, cheapest first:

```ts
import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";

const cake = (column: string) => pl.col("cake", column);
const running = pl.alias("running");

const rows = await pl.from(Cake.table)
  .filter(cake("bakery_id").eq(pl.literal(1)))
  .window(pl.over().by(cake("bakery_id")).sortBy(cake("price")).rows(null, 0), pl.sum(cake("price")).as(running))
  .sort(cake("price"))
  .select(cake("name"), running)
  .all(pool);

assert.deepEqual(
  rows.map((row) => [row.name, String(row.running)]),
  [["Lemon Drizzle", "3.20"], ["Chocolate Fudge", "7.70"], ["Chocolate Torte", "13.70"]],
);
```

## Composing pipelines

Because every step maps a relation to a relation, a function from a pipeline
to a pipeline is a reusable piece of a query, which any pipeline can apply:

```ts
import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";

/** The cakes a customer who avoids gluten can eat. */
function glutenFree(cakes: pl.Pipeline): pl.Pipeline {
  return cakes.filter(pl.col("cake", "gluten_free").eq(pl.literal(true)));
}

assert.equal(
  glutenFree(pl.from(Cake.table)).select(pl.col("cake", "name")).inspect().sql,
  "SELECT name FROM cake WHERE gluten_free = true",
);
```

A whole pipeline is a relation too: it can be the source of another, the
relation a join reads, or the other side of a set operation. Embedding one
takes it whole, values and all; its placeholders are numbered after the
consumer's, so the merged SQL and values still line up. Each cake's units
sold, from the order lines, joined back to the cake's name:

```ts
import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";

const line = (column: string) => pl.col("order_line", column);
const cake = (column: string) => pl.col("cake", column);
const sold = pl.alias("sold");

const units = pl.from(OrderLine.table)
  .group(line("cake_id"))
  .aggregate(pl.sum(line("quantity")).as(sold))
  .filterWith((binder) => sold.gte(binder.bind(1n)));

const report = pl.from(Cake.table)
  .filterWith((binder) => cake("bakery_id").eq(binder.bind(1)))
  .join(units, cake("id").eq(pl.alias("cake_id")))
  .sort(sold.desc())
  .select(cake("name"), sold);

assert.deepEqual(report.inspect().values.map((value) => value.value), [1n, 1n]);
assert.deepEqual(await report.all(pool), [
  { name: "Chocolate Fudge", sold: 2n },
  { name: "Lemon Drizzle", sold: 1n },
]);
```

An embedded pipeline's columns are referred to by their names, as
`alias("cake_id")` above. Where a name exists on both sides of a join,
`thatColumn(name)` names the joined relation's column and `thisColumn(name)`
the pipeline's own, as Rust's `that` and `this` do. A safe-integer `number`
binds as an `int8`, as every inferred parameter does ([Values and
types](13-values.md)), so `bind(1)` above sends the bakery's id as one, and
the values read back as `bigint`s.

`append` puts another relation's rows after this one's, matching columns by
position, and `distinct` after it makes the `UNION ALL` a `UNION`:

```ts
import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";

const everyone = pl.from(Bakery.table)
  .select(pl.col("bakery", "name"))
  .append(pl.from(Customer.table).select(pl.col("customer", "name")))
  .distinct();

const names = (await everyone.all(pool)).map((row) => row.name).sort();
assert.deepEqual(names, ["Kim", "Lee", "Moonlight", "Sunrise"]);
```

## When a pipeline fails

Building a pipeline cannot fail on what it will compile to: every method
appends one whole step. An argument of the wrong type, such as a model where a
source belongs or an expression where `take` wants a number, throws a
`TypeError` as the method is called. What can go wrong in the steps
themselves is found when the pipeline compiles, in `inspect()` or a terminal,
and is a `ConstructionError` thrown before anything is sent. Rust returns each
as a `PipelineError`; JavaScript's message says which:

- a name you gave collides with one PRQL defines;
- a table, column or alias name cannot be written safely as a name in SQL: it
  holds a double quote or a NUL byte, begins with `$`, or is `*`;
- the PRQL compiler refused the pipeline, such as a column read after a
  `select` that dropped it, with the compiler's own message.

```ts
import assert from "node:assert/strict";
import { ConstructionError, pipeline as pl } from "@necessary/pgorm";

const cake = (column: string) => pl.col("cake", column);

const dropped = pl.from(Cake.table).select(cake("name")).filter(cake("price").gt(pl.literal(4)));
assert.throws(() => dropped.inspect(), /PRQL compilation failed/);

const quoted = pl.from(Cake.table).select(pl.col("cake", 'na"me'));
assert.throws(() => quoted.inspect(), ConstructionError);
```

A name no step introduced and no source describes is not caught here: the
compiler has no catalog, so it takes the name as a column of the source, and
the server answers for it when the statement runs, with a `DatabaseError`.

A PRQL query written out whole as text compiles through the same compiler
with `prql(text, values)`, the run-time counterpart of Rust's `prql!` macro,
which holds its SQL to the grammar and its values to its placeholders
([Raw SQL](10-raw-sql.md)). The pipeline is for a query assembled from parts;
text is for one known whole.
