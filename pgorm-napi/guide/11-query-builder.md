# The query builder

pgorm-query builds SQL statements as values. A statement is a tree of tables,
columns and expressions, which renders to PostgreSQL text with every value
bound as a parameter. pgorm's models are written on top of it, and the
package exposes its builders directly: `select`, `insert`, `update`,
`deleteFrom` and `merge`, the expressions they hold, and the DDL builders of
[Schema generation](14-schema.md). Each JavaScript builder holds pgorm-query's
own statement, so it renders the SQL the Rust builder renders. This chapter
is the JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/10-query-builder.md).

The models of [Writing rows](04-writing.md) and [Reading rows](05-reading.md)
cover reading and writing one table's rows. Reach for the query builder when
a statement is about something else: a report joining several tables into
columns of your choosing, an `UPDATE` that reads another table, a `MERGE`, a
`WITH` query, a table function, or a table no model describes.

The builder refuses what PostgreSQL would refuse. Rust refuses most of it at
compile time, through its types; JavaScript refuses it as the builder is
called, an argument of a type a method does not take with a `TypeError` and a
statement that cannot be built with a [`ConstructionError`](../lib/index.d.ts),
so a statement that cannot run never exists. TypeScript's declarations refuse
much of it before then.

## A first statement

[`select`](../lib/select.d.ts) starts a `SELECT` of the items it is given. A
[`Table`](../lib/select.d.ts) names a table, and its `col` names one of its
columns, qualified by the table. Every builder is immutable: each method
returns a new statement from a copy of its receiver, so a statement is written
as one chain, and a base statement can be extended in two directions.

```ts
import assert from "node:assert/strict";
import { select, Table } from "@necessary/pgorm";

const cake = new Table("cake");
const bakery = new Table("bakery");
const query = select(cake.col("name"), bakery.col("city"))
  .from(cake)
  .join(bakery, cake.col("bakery_id").eq(bakery.col("id")))
  .where(cake.col("gluten_free").eq(true))
  .orderBy(cake.col("name").asc());

const { sql, values } = query.inspect();
assert.equal(
  sql,
  `SELECT "cake"."name", "bakery"."city" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" WHERE "cake"."gluten_free" = $1 ORDER BY "cake"."name" ASC`,
);
assert.deepEqual(values.map((value) => value.value), [true]);
```

`inspect()` writes `$1`, `$2`, … and returns the values beside the SQL, as
Rust's `build` does; that is what runs. Rust also renders a statement with
each value written into it as an escaped literal, its `to_string`, for tests
and logs. A JavaScript statement has no such rendering: `inspect()` is its
only one. [`literal`](../lib/expressions.d.ts) writes one value into the SQL
text, where a constant belongs in the statement itself.

A model's columns can stand in a statement too: `Cake.col("glutenFree")` is
the column `"cake"."gluten_free"`, and `Cake.table` the model's table. This
chapter names the bakery's tables and columns with `Table` and `col`, as a
program does for a table no model describes.

## Running a statement

A statement runs through the same terminals as SQL text
([Raw SQL](10-raw-sql.md)), given in place of the text: `query`, `one` and
`optional` resolve with rows, and `execute` with the number of rows written.
Rust decodes a `SELECT`'s rows through `into_tuple` and `into_model`, and runs
a write through `execute_raw` with its values wrapped; JavaScript runs either
as it is, each row a plain object keyed by column name:

```ts
import assert from "node:assert/strict";
import { col, Decimal, deleteFrom, select, Table, update } from "@necessary/pgorm";

const cake = new Table("cake");
const bakery = new Table("bakery");
const query = select(cake.col("name"), bakery.col("city"))
  .from(cake)
  .join(bakery, cake.col("bakery_id").eq(bakery.col("id")))
  .where(cake.col("gluten_free").eq(true))
  .orderBy(cake.col("name").asc());
assert.deepEqual(await pool.query(query), [
  { name: "Chocolate Torte", city: "Oslo" },
  { name: "Lemon Drizzle", city: "Oslo" },
  { name: "Vanilla Sponge", city: "Bergen" },
]);

const raise = update(cake).set("price", col("price").add(new Decimal("0.50"))).where(col("bakery_id").eq(2));
assert.equal(raise.inspect().sql, `UPDATE "cake" SET "price" = "price" + $1 WHERE "bakery_id" = $2`);
assert.equal(await pool.execute(raise), 2);

const deleted = await pool.query(deleteFrom(cake).where(col("id").eq(5)).returning([col("name")]));
assert.deepEqual(deleted, [{ name: "Chocolate Torte" }]);
```

## Where a bound value's type comes from

Each value a statement binds is a parameter, and PostgreSQL decides its type
from where it stands. Beside a column it takes the column's type, which is
the common case: `"price" + $1` reads `$1` as a `numeric`. Where nothing
around it says, the server cannot choose: a value in a `SELECT` list or a
`VALUES` row, the argument of a function that takes any array such as
`unnest`, a number added to a date. There, bind the value with
[`bind`](../lib/expressions.d.ts) and cast it with `cast`, which also tells the
server the type the value is sent as, `CAST($1::int8 AS integer)`. The
examples under [WITH](#with), [Table functions](#table-functions) and
[Ranges](#ranges) do.

## Names

A table, a column, an alias and a type are *names*, and the builder writes
every name as a quoted identifier: `new Table("order")` is `"order"`, so a
reserved word needs no care, and a name with a dot in it is one name. A name
is a string of 1 to 63 bytes, the most PostgreSQL keeps of an identifier; a
longer one is a `ConstructionError` rather than a name the server would cut
short.

Rust takes a name only as a name type, never as a `&str`, so that no text
from a user can become an identifier by accident, and has four doors into
one: an entity's own names, a type deriving `SqlName` or `StaticName`,
`alias`, and `Name::runtime` for a name computed while the program runs.
JavaScript keeps names and values apart by position instead. Where a method
takes a name, a string is that name and nothing else; where it takes a value,
a string is bound as a parameter; and where it takes an expression, a string
is refused with a `TypeError`, so text cannot stand in for SQL anywhere. A
name computed at run time, such as a table chosen by configuration, is a
string like any other:

```ts
import assert from "node:assert/strict";
import { call, col, select, Table } from "@necessary/pgorm";

// A table loaded by a nightly job, which no model describes.
const priceList = new Table("price_list");
const query = select(call("sum", col("new_price")).as("total")).from(priceList);
assert.equal(query.inspect().sql, `SELECT SUM("new_price") AS "total" FROM "price_list"`);

const year = 2025;
const archive = select(col("name")).from(new Table(`price_list_${year}`));
assert.equal(archive.inspect().sql, `SELECT "name" FROM "price_list_2025"`);

// A string where an expression belongs is refused, not written into the SQL.
// @ts-expect-error: a projection is an expression
assert.throws(() => select("name"), TypeError);
```

A `Table` takes `{ schema, alias }`, and `col(name, { table, schema })` names
a column of a table given by name. `.as(alias)` names a projection's output
column.

## Expressions

The expression builders write what a statement holds:
[`col`](../lib/expressions.d.ts) for a column, `bind` for a value, and methods
of every expression for the operators: `eq`, `gt`, `between`, `isIn`,
`isNull`, `add`, `mul`, `and`, `or`, `like`, and many more. An operand that is
not an expression is bound as a parameter. [`Condition.all`](../lib/expressions.d.ts)
and `Condition.any` group conditions under `AND` and `OR`.
[`call`](../lib/expressions.d.ts) calls a function: the aggregates,
`coalesce`, `lower`, `upper`, `round`, the UUID functions and the
set-returning ones. [`caseWhen`](../lib/expressions.d.ts) writes a searched
`CASE`, and `caseOf` the simple form, comparing one operand with each arm's
value.

```ts
import assert from "node:assert/strict";
import { call, caseWhen, col, Condition, likeExpr, select, Table } from "@necessary/pgorm";

const query = select(
  col("name"),
  call("lower", col("name")).as("key"),
  caseWhen(col("price").gte(5), "premium").when(col("price").gte(3), "standard").else("budget").as("tier"),
)
  .from(new Table("cake"))
  .where(Condition.any(
    col("name").like(likeExpr("%Choc%")),
    Condition.all(col("gluten_free").eq(true), col("price").between(2, 4)),
  ))
  .orderBy(col("name").asc());

assert.equal(
  query.inspect().sql,
  [
    `SELECT "name", LOWER("name") AS "key",`,
    `(CASE WHEN ("price" >= $1) THEN $2 WHEN ("price" >= $3) THEN $4 ELSE $5 END) AS "tier"`,
    `FROM "cake"`,
    `WHERE "name" LIKE $6 OR ("gluten_free" = $7 AND "price" BETWEEN $8 AND $9)`,
    `ORDER BY "name" ASC`,
  ].join(" "),
);

const tiers = await pool.query(query);
assert.deepEqual(
  tiers.map((row) => [row.name, row.tier]),
  [
    ["Chocolate Fudge", "standard"],
    ["Chocolate Torte", "premium"],
    ["Lemon Drizzle", "standard"],
    ["Vanilla Sponge", "budget"],
  ],
);
```

`call` reaches the functions pgorm-query has a constructor for, and refuses
any other name with a `ConstructionError`.

**Not available in JavaScript yet:** calling any function by name, which
Rust's `Func::named` does, and writing a piece of SQL the builder has no
method for, with `$1`, `$2`, … for its bound values, which Rust's
`Expr::template` does.

A few expressions take a little more care:

- A `LIKE` pattern given as a string is bound, its `%` and `_` wildcards and
  its backslash escaping them. [`likeExpr`](../lib/expressions.d.ts) makes a
  pattern whose `escape` picks another escape character, or `noEscape` none.
  A pattern holding text from a user must have that text's wildcards escaped
  first; `startsWith`, `endsWith` and `containsText` read their argument as
  text, with no wildcards in it ([Reading rows](05-reading.md)).
- `cast(type)` writes `CAST(.. AS type)`, the type a name or a
  [`TypeName`](../lib/index.d.ts), `{ array: true }` for an array of it. A cast
  is an expression like any other, so it compares and builds on as one.
- `collate(collation)` puts text under a collation, which decides how it
  compares and sorts.
- `at` and `slice` subscript an array, counting from 1.
- `isIn` and `isNotIn` take a list of operands or a subquery, and
  [`exists`](../lib/expressions.d.ts) tests a subquery; a `Select`'s `as(alias)`
  reads one as a table.

```ts
import assert from "node:assert/strict";
import { col, select, Table } from "@necessary/pgorm";

const query = select(col("name"))
  .from(new Table("cake"))
  .where(col("id").isIn(select(col("cake_id")).from(new Table("order_line")).where(col("quantity").gt(1))))
  .where(col("price").cast("integer").lt(10))
  .orderBy(col("name").collate("C").asc());

assert.equal(
  query.inspect().sql,
  [
    `SELECT "name" FROM "cake"`,
    `WHERE "id" IN (SELECT "cake_id" FROM "order_line" WHERE "quantity" > $1)`,
    `AND CAST("price" AS integer) < $2`,
    `ORDER BY ("name" COLLATE "C") ASC`,
  ].join(" "),
);
assert.deepEqual(await pool.query(query), [{ name: "Chocolate Fudge" }]);
```

## INSERT, ON CONFLICT and RETURNING

[`insert`](../lib/writes.d.ts) names its table, then its columns once, then
takes rows: `values` takes one row, an operand for each column, and refuses a
row of another length with a `ConstructionError`. `select(query)` inserts a
query's rows, and `defaultValues()` one row of defaults.

`onConflict` takes a [`Conflict`](../lib/writes.d.ts): the columns or
constraint that conflict, and what to do. `Conflict.on(..)` names the
conflicting columns, `Conflict.onConstraint(name)` the constraint, and
`Conflict.doNothing()` skips a conflicting row whatever it conflicts on. A
target is followed by `doNothing()`, `update(column, ..)`, which takes each
column's new value from the row that was refused, `excluded`, or `set(column,
value)`. A target named by columns takes a `where` before its action, the
predicate of a partial unique index, and an update takes one after it,
choosing which conflicting rows to update. A sign-up form that may name an
existing customer:

```ts
import assert from "node:assert/strict";
import { col, Conflict, ConstructionError, insert, Table } from "@necessary/pgorm";

const customer = new Table("customer");
const signUp = (name: string, email: string) =>
  insert(customer)
    .columns("name", "email")
    .values(name, email)
    .onConflict(Conflict.on("email").update("name"))
    .returning([col("id")]);

assert.equal(
  signUp("Kimberly", "kim@example.com").inspect().sql,
  `INSERT INTO "customer" ("name", "email") VALUES ($1, $2) ON CONFLICT ("email") DO UPDATE SET "name" = "excluded"."name" RETURNING "id"`,
);

// Kim's address is taken, so her row is updated and keeps its id.
assert.deepEqual(await pool.one(signUp("Kimberly", "kim@example.com")), { id: 1 });
// A new address is a new customer. Its id is 4, not 3: the insert that
// conflicted drew 3 from the id's sequence before it conflicted.
assert.deepEqual(await pool.one(signUp("Mo", "mo@example.com")), { id: 4 });

const kim = await pool.one("SELECT name FROM customer WHERE id = 1");
assert.equal(kim.name, "Kimberly");

// A row of one value, for two columns.
assert.throws(() => insert(customer).columns("name", "email").values("Mo"), ConstructionError);
```

`returning(items)` reads the rows a write touched: the items given, or every
column when there are none. PostgreSQL 18 also lets a `RETURNING` list read the
row as it was before the write and as it is after it, which
[`ReturningRow`](../lib/writes.d.ts)`.old` and `.new` name:

```ts
import assert from "node:assert/strict";
import { col, ReturningRow, Table, update } from "@necessary/pgorm";

const doubled = update(new Table("cake"))
  .set("price", col("price").mul(2))
  .where(col("id").eq(1))
  .returning([ReturningRow.old.col("price").as("was"), ReturningRow.new.col("price").as("now")]);

assert.equal(
  doubled.inspect().sql,
  `UPDATE "cake" SET "price" = "price" * $1 WHERE "id" = $2 RETURNING old."price" AS "was", new."price" AS "now"`,
);
const [row] = await pool.query(doubled);
assert.deepEqual([String(row?.was), String(row?.now)], ["4.50", "9.00"]);
```

A write with a `RETURNING` list is a statement whose rows a `WITH` clause can
read, and every write takes `with(..)`.

## UPDATE … FROM and DELETE … USING

An `UPDATE` can read other tables with `from`, and a `DELETE` with `using`,
each joined in the `WHERE` clause. Neither takes `ORDER BY` or `LIMIT`, which
PostgreSQL refuses on both; to write some rows and not others, filter on a
subquery. An `UPDATE` or `DELETE` runs only once it has a `where`, or
`allRows()` saying it means every row; without either it is a
`ConstructionError` when it is inspected or run, so no statement writes every
row by omission.

```ts
import assert from "node:assert/strict";
import { col, ConstructionError, Decimal, deleteFrom, Table, update } from "@necessary/pgorm";

const cake = new Table("cake");
const bakery = new Table("bakery");

// Every cake sold in Bergen goes up by fifty cents.
const raise = update(cake)
  .set("price", cake.col("price").add(new Decimal("0.50")))
  .from(bakery)
  .where(cake.col("bakery_id").eq(bakery.col("id")))
  .where(bakery.col("city").eq("Bergen"));
assert.equal(
  raise.inspect().sql,
  `UPDATE "cake" SET "price" = "cake"."price" + $1 FROM "bakery" WHERE "cake"."bakery_id" = "bakery"."id" AND "bakery"."city" = $2`,
);
assert.equal(await pool.execute(raise), 2);

// Lee's order lines go.
const orderLine = new Table("order_line");
const order = new Table("order");
const forget = deleteFrom(orderLine)
  .using(order)
  .where(orderLine.col("order_id").eq(order.col("id")))
  .where(order.col("customer_id").eq(2));
assert.equal(
  forget.inspect().sql,
  `DELETE FROM "order_line" USING "order" WHERE "order_line"."order_id" = "order"."id" AND "order"."customer_id" = $1`,
);
assert.equal(await pool.execute(forget), 1);

// A delete that says nothing of which rows is refused.
assert.throws(() => deleteFrom(orderLine).inspect(), ConstructionError);
```

## MERGE

`MERGE` writes a table from a second relation, row by row: each row of the
target the condition pairs with a source row is *matched*, each source row
with no target row is *not matched*, and each target row with no source row
is *not matched by source*. Each kind takes its own arms, each arm an action
and an optional condition of its own, and a row takes the first arm whose
condition holds. A matched row can be updated or deleted, an unmatched
source row inserted.

[`merge`](../lib/merge.d.ts) takes the target, the source and the condition,
and gives a `PendingMerge`, which is no statement: PostgreSQL refuses a
`MERGE` with no `WHEN` arm, so it has no `inspect()`, TypeScript refuses to
run it, and every terminal refuses it with a `ConstructionError`. Its first
arm gives the `Merge`. [`MergeAction`](../lib/merge.d.ts) makes each arm's
action: `update` and `delete` for a target row, `insert` and
`insertDefaults` for a source row, `doNothing` for either. Rust refuses an arm
that cannot apply to its kind of row, such as an insert for a matched row, at
compile time; TypeScript's declarations refuse it too, and the builder throws
a `ConstructionError` for one it is given anyway.

Here the nightly price list for Sunrise holds two rows, Chocolate Fudge at
4.80 and Lemon Drizzle at 3.20. A cake on the list whose price differs takes
the new price, and a Sunrise cake missing from the list is withdrawn:

```ts
import assert from "node:assert/strict";
import { ConstructionError, merge, MergeAction, Table } from "@necessary/pgorm";

await pool.execute("CREATE TABLE price_list (name text NOT NULL, new_price numeric(10, 2) NOT NULL)");
await pool.execute("INSERT INTO price_list VALUES ('Chocolate Fudge', 4.80), ('Lemon Drizzle', 3.20)");

const cake = new Table("cake");
const priceList = new Table("price_list");
const pending = merge(cake, priceList, cake.col("name").eq(priceList.col("name")));
// @ts-expect-error: a MERGE with no WHEN arm is no statement
await assert.rejects(pool.query(pending), ConstructionError);

const reprice = pending
  .whenMatched(MergeAction.update("price", priceList.col("new_price")), {
    condition: cake.col("price").ne(priceList.col("new_price")),
  })
  .whenNotMatchedBySource(MergeAction.delete(), { condition: cake.col("bakery_id").eq(1) })
  .returningAction()
  .returning([cake.col("name")]);

assert.equal(
  reprice.inspect().sql,
  [
    `MERGE INTO "cake" USING "price_list" ON "cake"."name" = "price_list"."name"`,
    `WHEN MATCHED AND "cake"."price" <> "price_list"."new_price" THEN UPDATE SET "price" = "price_list"."new_price"`,
    `WHEN NOT MATCHED BY SOURCE AND "cake"."bakery_id" = $1 THEN DELETE`,
    `RETURNING merge_action(), "cake"."name"`,
  ].join(" "),
);

const changes = (await pool.query(reprice)).map((row) => [row.merge_action, row.name]).sort();
assert.deepEqual(changes, [
  ["DELETE", "Chocolate Torte"],
  ["UPDATE", "Chocolate Fudge"],
]);
```

Lemon Drizzle matched but its price had not changed, and no arm applied, so
it was left as it was. `returningAction()` adds `merge_action()`, the action
each row took, as the first column. The source can be any relation a `FROM`
takes, a subquery among them; a value in the source is bound like any other,
and since the source assigns it to no column, it may need a cast to the type
it stands for. `only()` writes `ONLY` before the target.

## WITH

A [`With`](../lib/select.d.ts) clause names queries a statement reads like
tables, each a common table expression, and `cte` adds more. Its query can be
a `SELECT` or a write with a `RETURNING` list, which then runs once and hands
its rows to the rest of the statement. Each takes `{ columns, materialized }`.

```ts
import assert from "node:assert/strict";
import { call, col, select, Table, With } from "@necessary/pgorm";

const cake = new Table("cake");
const sold = new Table("sold");
const query = select(cake.col("name"), sold.col("units"))
  .from(cake)
  .join(sold, cake.col("id").eq(sold.col("cake_id")))
  .orderBy(cake.col("name").asc())
  .with(new With(
    "sold",
    select(col("cake_id"), call("sum", col("quantity")).as("units")).from(new Table("order_line")).groupBy(col("cake_id")),
  ));

assert.equal(
  query.inspect().sql,
  [
    `WITH "sold" AS (SELECT "cake_id", SUM("quantity") AS "units" FROM "order_line" GROUP BY "cake_id")`,
    `SELECT "cake"."name", "sold"."units" FROM "cake"`,
    `INNER JOIN "sold" ON "cake"."id" = "sold"."cake_id" ORDER BY "cake"."name" ASC`,
  ].join(" "),
);
assert.deepEqual(await pool.query(query), [
  { name: "Carrot Cake", units: 1n },
  { name: "Chocolate Fudge", units: 2n },
  { name: "Lemon Drizzle", units: 1n },
]);
```

[`With.recursive`](../lib/select.d.ts) holds one query that reads its own
rows: a starting query, `UNION ALL`, and a query over the rows so far,
repeated until it adds none. It also takes `search` and `cycle`, for walking
graphs. The days from the first of November to the fourth, each with the
orders placed on it, the days without an order included:

```ts
import assert from "node:assert/strict";
import { bind, call, col, select, Table, With } from "@necessary/pgorm";

const days = new Table("days");
const order = new Table("order");
const calendar = select(bind(Temporal.PlainDate.from("2026-11-01")).cast("date").as("day"))
  .unionAll(
    select(col("day").add(bind(1).cast("integer")))
      .from(days)
      .where(col("day").lt(bind(Temporal.PlainDate.from("2026-11-04")).cast("date"))),
  );

const query = select(days.col("day"), call("count", order.col("id")).as("orders"))
  .from(days)
  .join(order, order.col("placed_at").cast("date").eq(days.col("day")), { kind: "left" })
  .groupBy(days.col("day"))
  .orderBy(days.col("day").asc())
  .with(With.recursive("days", calendar, { columns: ["day"] }));

const rows = await pool.query(query);
assert.deepEqual(
  rows.map((row) => [String(row.day), row.orders]),
  [["2026-11-01", 0n], ["2026-11-02", 1n], ["2026-11-03", 1n], ["2026-11-04", 0n]],
);
```

The starting row's date and the `1` added to each day are cast because
nothing around them gives them a type: the first stands in a `SELECT` list,
and the second beside a date, which PostgreSQL can add an integer or an
interval to. A day reads as a `Temporal.PlainDate`.

## Window functions

A window function computes a value for each row from the rows around it.
`over` puts a function call over a [`Window`](../lib/windows.d.ts), which
partitions the rows, orders them within each partition, and can narrow them
to a frame: [`FrameType`](../lib/windows.d.ts)`.rows`, `.range` or `.groups`
starts one, and its bounds follow. [`windowFunction`](../lib/windows.d.ts)
makes `row_number`, `rank`, `lag` and the other functions only a window
takes. Each cake beside the running total of its bakery's prices, cheapest
first:

```ts
import assert from "node:assert/strict";
import { call, col, FrameType, select, Table, Window } from "@necessary/pgorm";

const running = new Window()
  .partitionBy(col("bakery_id"))
  .orderBy(col("price").asc())
  .frame(FrameType.rows.unboundedPreceding().andCurrentRow());
const query = select(col("name"), call("sum", col("price")).over(running).as("running"))
  .from(new Table("cake"))
  .where(col("bakery_id").eq(1))
  .orderBy(col("price").asc());

assert.equal(
  query.inspect().sql,
  [
    `SELECT "name", SUM("price") OVER ( PARTITION BY "bakery_id" ORDER BY "price" ASC`,
    `ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW ) AS "running"`,
    `FROM "cake" WHERE "bakery_id" = $1 ORDER BY "price" ASC`,
  ].join(" "),
);

const rows = await pool.query(query);
assert.deepEqual(rows.map((row) => [row.name, String(row.running)]), [
  ["Lemon Drizzle", "3.20"],
  ["Chocolate Fudge", "7.70"],
  ["Chocolate Torte", "13.70"],
]);
```

A window used by several columns can be named once in the statement's
`WINDOW` clause, with the statement's `window(name, window)`, and each column
refers to it by name, `over("name")`; an `ExtendedWindow` builds on a named
one. The [pipeline](12-pipeline.md) writes windows in fewer lines, when its
shape fits.

## Grouping sets

A `GROUP BY` list can group by several sets of columns in one query.
[`GroupingElement`](../lib/select.d.ts) builds them: `rollup` for subtotals
down a hierarchy, `cube` for every combination, and `sets` for sets of your
choosing. A row grouped by a set that leaves a column out holds `NULL` there.
The cakes counted by bakery and by whether they are gluten-free, with a
subtotal per bakery and a grand total:

```ts
import assert from "node:assert/strict";
import { call, col, GroupingElement, select, Table } from "@necessary/pgorm";

const query = select(col("bakery_id"), col("gluten_free"), call("count", col("id")).as("cakes"))
  .from(new Table("cake"))
  .groupBy(GroupingElement.rollup(col("bakery_id"), col("gluten_free")))
  .orderBy(col("bakery_id").asc(), col("gluten_free").asc());

assert.equal(
  query.inspect().sql,
  [
    `SELECT "bakery_id", "gluten_free", COUNT("id") AS "cakes" FROM "cake"`,
    `GROUP BY ROLLUP ("bakery_id", "gluten_free")`,
    `ORDER BY "bakery_id" ASC, "gluten_free" ASC`,
  ].join(" "),
);

const rows = await pool.query(query);
assert.deepEqual(rows.map((row) => [row.bakery_id, row.gluten_free, row.cakes]), [
  [1, false, 1n],
  [1, true, 2n],
  [1, null, 3n],
  [2, false, 1n],
  [2, true, 1n],
  [2, null, 2n],
  [null, null, 5n],
]);
```

**Not available in JavaScript yet:** `GROUPING(..)`, which Rust's
`Func::grouping` writes to tell the `NULL` a grouping set leaves from a
`NULL` in the data.

## Table functions

A function that returns rows, such as `generate_series` or `unnest`, can be
read as a table. [`tableFunction`](../lib/select.d.ts) wraps the call, and
`TableFunction.rowsFrom` several read side by side. `columns` names its
columns, `withOrdinality` adds a last column numbering the rows from 1, and
`alias` finishes it as something `from` and the joins take. `call` reaches
`generate_series`, `unnest` and the JSON record functions.

Unnesting a list of ids with its ordinal reads rows back in the order the
program asked for them. `unnest` takes an array of any type, so the bound
array is cast to say which:

```ts
import assert from "node:assert/strict";
import { bind, call, select, Table, tableFunction } from "@necessary/pgorm";

const cake = new Table("cake");
const wanted = tableFunction(call("unnest", bind([4, 1, 3]).cast("integer", { array: true })))
  .withOrdinality()
  .columns("id", "position")
  .alias("wanted");
const query = select(cake.col("name"))
  .from(wanted)
  .join(cake, cake.col("id").eq(wanted.col("id")))
  .orderBy(wanted.col("position").asc());

assert.equal(
  query.inspect().sql,
  [
    `SELECT "cake"."name" FROM UNNEST(CAST($1::int8[] AS integer[])) WITH ORDINALITY AS "wanted"("id", "position")`,
    `INNER JOIN "cake" ON "cake"."id" = "wanted"."id" ORDER BY "wanted"."position" ASC`,
  ].join(" "),
);
assert.deepEqual(await pool.query(query), [
  { name: "Vanilla Sponge" },
  { name: "Chocolate Fudge" },
  { name: "Carrot Cake" },
]);
```

A JavaScript number that is a safe integer is sent as an `int8`, so the array
travels as `int8[]` and the cast says what it stands for.

## SQL/JSON

PostgreSQL's SQL/JSON functions query a JSON document with a path:
[`jsonExists`](../lib/json.d.ts) tests that the path finds something,
`jsonValue` reads one scalar, `jsonQuery` reads JSON, and `jsonTable` turns
what a path finds into rows. The constructors `jsonObject`, `jsonArray`,
`jsonObjectagg` and `jsonArrayagg` build JSON, and `isJson` tests text. A
path is bound as a value, except in `JSON_TABLE`, whose path the server takes
only as a literal; a value the path needs is passed with `passing`, and bound.

An order arriving as JSON, read as rows of cake and quantity. A plain object
binds as JSON:

```ts
import assert from "node:assert/strict";
import { bind, jsonTable, JsonTableColumn, select } from "@necessary/pgorm";

const incoming = {
  customer: "kim@example.com",
  lines: [{ cake: 2, qty: 3 }, { cake: 4, qty: 1 }],
};

const line = jsonTable(bind(incoming), "$.lines[*]", [
  JsonTableColumn.value("cake_id", "integer", { path: "$.cake" }),
  JsonTableColumn.value("quantity", "integer", { path: "$.qty" }),
], { alias: "line" });
const query = select().from(line);

assert.equal(
  query.inspect().sql,
  [
    `SELECT * FROM JSON_TABLE($1::jsonb, '$.lines[*]' COLUMNS`,
    `("cake_id" integer PATH '$.cake', "quantity" integer PATH '$.qty')) AS "line"`,
  ].join(" "),
);
assert.deepEqual(await pool.query(query), [
  { cake_id: 2, quantity: 3 },
  { cake_id: 4, quantity: 1 },
]);
```

`contains` is `@>` on a `jsonb` as on a range or an array, `matches` is `@@`
and `pathExists` is `@?`.

**Not available in JavaScript yet:** the other `jsonb` operators, `->`,
`->>`, `?` and their kin, which Rust's `get_json_field`, `cast_json_field`
and `has_json_key` write.

## Ranges

A [`Range`](../lib/index.d.ts) binds as one of PostgreSQL's range types, and
the range operators are methods of every expression: `contains` (`@>`),
`containedBy` (`<@`), `overlaps` (`&&`) and the positions. A range's kind is
never inferred from its bounds, so a range binds as a
[`Value`](../lib/index.d.ts) naming its type. Here the range stands on its own
beside a timestamp, which many operators accept, so it is cast to
`tstzrange` as well. The orders placed on the second of November:

```ts
import assert from "node:assert/strict";
import { bind, col, Range, select, Table, Value } from "@necessary/pgorm";

const secondOfNovember = new Range(
  Temporal.Instant.from("2026-11-02T00:00:00Z"),
  Temporal.Instant.from("2026-11-03T00:00:00Z"),
);
const query = select(col("id"))
  .from(new Table("order"))
  .where(bind(new Value(secondOfNovember, "tstzrange")).cast("tstzrange").contains(col("placed_at")));

assert.equal(
  query.inspect().sql,
  `SELECT "id" FROM "order" WHERE CAST($1::tstzrange AS tstzrange) @> "placed_at"`,
);
assert.deepEqual(await pool.query(query), [{ id: 1 }]);
```

[Values and types](13-values.md) covers ranges and multiranges as column
types, including range types a schema creates.

## Locking

A `SELECT` can lock the rows it reads: `lock("update")` for `FOR UPDATE`,
`"noKeyUpdate"`, `"share"` and `"keyShare"` for the weaker strengths, `of`
for the tables to lock, and `wait: "nowait"` or `"skipLocked"`, the usual
way for several workers to take jobs from one queue table. A lock lasts until
the transaction ends, so it belongs inside one ([Transactions](08-transactions.md)).

```ts
import assert from "node:assert/strict";
import { col, select, Table } from "@necessary/pgorm";

const next = select(col("id"))
  .from(new Table("order"))
  .where(col("bakery_id").eq(1))
  .lock("update", { wait: "skipLocked" });

assert.equal(next.inspect().sql, `SELECT "id" FROM "order" WHERE "bakery_id" = $1 FOR UPDATE SKIP LOCKED`);

const taken = await pool.transaction((tx) => tx.query(next));
assert.deepEqual(taken, [{ id: 1 }]);
```

## What else the builders write

The JavaScript builders write a few things the Rust guide's chapter does not
show, each the pgorm-query construct of the same name:

- `limitWithTies(n)` on a `SELECT` is `FETCH FIRST n ROWS WITH TIES`, in
  `LIMIT`'s place, its count bound: the first `n` rows, and every row that
  ties with the last of them in the ordering. A tie limit with no `ORDER BY`,
  or beside `SKIP LOCKED`, is refused before anything is sent.
- `table.tablesample("bernoulli" | "system", percentage, { repeatable })`
  reads a sample of a table's rows, as a `FROM` item.
- `regex`, `notRegex`, `iregex` and `notIregex` are `~`, `!~`, `~*` and
  `!~*`, a POSIX regular expression, bound unless it is an expression.
- `overriding("systemValue" | "userValue")` on an `INSERT` writes a value
  into an identity column, or lets the identity choose it over the one
  given.

```ts
import assert from "node:assert/strict";
import { col, select, Table } from "@necessary/pgorm";

// The dearest cakes, the second-dearest's ties included.
const dearest = select(col("name"))
  .from(new Table("cake"))
  .orderBy(col("price").desc())
  .limitWithTies(2);
assert.equal(
  dearest.inspect().sql,
  `SELECT "name" FROM "cake" ORDER BY "price" DESC FETCH FIRST $1 ROWS WITH TIES`,
);
assert.deepEqual((await pool.query(dearest)).map((row) => row.name), ["Chocolate Torte", "Chocolate Fudge"]);

const chocolate = select(col("name"))
  .from(new Table("cake"))
  .where(col("name").iregex("^choc"))
  .orderBy(col("name").asc());
assert.deepEqual((await pool.query(chocolate)).map((row) => row.name), ["Chocolate Fudge", "Chocolate Torte"]);

// Every row, sampled at 100 percent with a fixed seed.
const sampled = select(col("name")).from(new Table("cake").tablesample("bernoulli", 100, { repeatable: 7 }));
assert.equal((await pool.query(sampled)).length, 5);
```
