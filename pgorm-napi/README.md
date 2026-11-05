# pgorm-napi

pgorm from Node.js and Deno: a Node-API addon built with
[Neon](https://neon-bindings.com), and the ES module that loads it. It is the
JavaScript counterpart of `pgorm-python`, and like it lives in its own Cargo
workspace so that ordinary pgorm builds never meet it. The rules it follows
are [docs/spec/napi.md](../docs/spec/napi.md).

The addon loads in both runtimes, runs pgorm's asynchronous work on a tokio
runtime of its own, settles a Promise with the outcome, and lets the process
exit when its work is done. JavaScript connects through pools, runs bound SQL,
scopes transactions and savepoints, streams rows, runs COPY and listens for
notifications ([Connections](#connections)), builds pgorm-query's statements
([Statements](#statements)), DDL ([Schema](#schema)) and pipelines
([Pipelines](#pipelines)), declares models and reads and writes them, their
relations and graphs ([Models](#models)), and every value pgorm holds crosses
into and out of JavaScript with a declared type ([Values](#values)). It needs
a runtime with `Temporal` as a global: Node.js 26 or later, or Deno 2.9.5 or
later.

## Installing

pgorm-napi is packaged for npm: a main package, `@necessary/pgorm`, of
JavaScript and declarations, and a package per platform holding its prebuilt
addon, `@necessary/pgorm-<os>-<cpu>[-<libc>]`, which npm and Deno install
beside it — macOS arm64 and Linux x86-64 (glibc) today.

```sh
npm install @necessary/pgorm
```

```json
{
  "nodeModulesDir": "auto",
  "imports": { "@necessary/pgorm": "npm:@necessary/pgorm@^0.2.0" }
}
```

The second is a Deno project's `deno.json`; `deno install` then fetches the
packages. CI builds the packages, installs them in both runtimes and keeps the
tarballs, but nothing is published to npm yet; meanwhile, build from a
checkout. [DISTRIBUTION.md](DISTRIBUTION.md)
covers the packages, the platforms, what Deno's `--allow-ffi` grants, the
dependency notices and how the packages are built and checked.

## Building

```sh
node pgorm-napi/scripts/build.mjs            # debug build
node pgorm-napi/scripts/build.mjs --release  # release build, as the packages ship it
```

The script runs Cargo (into the repository's `target/` unless
`CARGO_TARGET_DIR` says otherwise) and copies the library it produces to
`pgorm-napi/lib/pgorm_napi.node`, which git ignores. A release build uses the
crate's `addon` profile, Cargo's release profile with fat LTO, one codegen
unit and the symbol table stripped: about 11.8 MB on macOS arm64 rather than
23 MB, at the cost of a longer link
([DISTRIBUTION.md](DISTRIBUTION.md#the-addons-size)). The addon targets
Node-API 6 and resolves its Node-API symbols from whichever runtime loads it,
so one build serves both. The module loads a library it finds there before
looking for an installed platform package, so a checkout runs its own build.

## Loading

Import the module, never the `.node` file — `@necessary/pgorm` once
installed, as the examples here do, or `./pgorm-napi/lib/index.js` from a
checkout:

```js
import { DatabaseError, Pool } from "@necessary/pgorm";

await using pool = new Pool("postgres://postgres@localhost/postgres?sslmode=disable");
try {
  const row = await pool.one("SELECT $1::int8 + 1 AS n", [41]);
  console.log(row.n); // 42n
} catch (error) {
  if (error instanceof DatabaseError) console.error(error.sqlstate, error.message);
  else throw error;
}
```

- **Node.js** loads it as is: `node app.js`.
- **Deno** needs `--allow-ffi`, to open a native library, and `--allow-read`,
  to resolve its path: `deno run --allow-ffi --allow-read app.ts`. Native code
  runs outside Deno's permission checks, so `--allow-net` is neither needed
  nor a limit on what the addon reaches; granting `--allow-ffi` to a program
  that loads it trusts it with everything the process can do.

`lib/index.d.ts` declares every export, and Deno finds it through the module's
`@ts-self-types` directive.

## Connections

```js
import { connect } from "@necessary/pgorm";

await using pool = await connect(process.env.DATABASE_URL, { maxSize: 10 });

await pool.execute("UPDATE accounts SET visits = visits + 1 WHERE id = $1", [7]);  // affected rows
const rows = await pool.query("SELECT id, name FROM accounts WHERE id = ANY($1)", [[1, 2, 3]]);
const row = await pool.one("SELECT now() AS at");               // exactly one row
const maybe = await pool.optional("SELECT 1 WHERE false");       // null

await pool.transaction(async (tx) => {                           // commits, or rolls back on throw
  await tx.execute("INSERT INTO audit VALUES ($1)", ["visit"]);
  await tx.transaction(async (savepoint) => { /* rolled back alone if it throws */ });
});

for await (const account of pool.stream("SELECT * FROM accounts")) {
  if (account.id === 100) break;                                // releases the connection
}

await pool.execute("SELECT pg_sleep(10)", [], { signal: AbortSignal.timeout(1000) });  // rejects after 1 s
```

- **`Pool`** builds from a connection string and options (`tls`, `ca`,
  `maxSize`, `connectTimeout`, `acquireTimeout`, `statementCacheSize`,
  `recycle`, `statementCacheChannel`; durations in milliseconds) without
  sending anything; `connect` also pings. TLS is `verify-full` — certificate
  and host name checked against `ca` or the platform's trust store — unless
  the string says
  `sslmode=disable` or `tls` is `"disable"`, and never falls back to plaintext.
  A statement run on a pool takes a connection for itself alone. After DDL
  that a cached plan does not see, such as a domain's changed default,
  `pool.clearStatementCaches()` makes every connection prepare its statements
  again: an idle one before it is next lent, one checked out at the time once
  it is returned and lent again. That reaches only this process's pool; one
  built with `statementCacheChannel: STATEMENT_CACHE_CHANNEL` also clears on
  every notification there, as pgorm-migration's runner sends after a
  migration from any process. It listens through a connection of its own,
  opened by its first acquisition, reopened if it drops (clearing once each
  time it listens again) and closed with the pool.
- **`Connection`** (from `pool.acquire()`) is the caller's until closed, and
  runs one operation at a time; another meanwhile is a `LifecycleError`, not a
  queue.
- **`Transaction`** exists only inside `transaction(fn)`, as a Rust
  transaction exists only inside its closure: it commits when `fn` resolves
  and rolls back when it throws, rejecting with `fn`'s own error, so throwing
  is how a callback rolls back. A commit that fails rejects with its failure,
  and a ROLLBACK that fails with a `SuppressedError` whose `error` is the
  failure and whose `suppressed` is what `fn` threw; either discards the
  connection. On a transaction, `transaction(fn)` opens a savepoint. A
  transaction borrows its parent exclusively, as in Rust: while a savepoint
  is open its transaction refuses statements and other savepoints, and while
  a transaction is open its connection refuses other work — refused with a
  `LifecycleError`, never raced. `mode` (`"readWrite"`, `"readOnly"`,
  `"deferrable"`) and `isolation` choose how it opens.
- **`execute`, `query`, `one`, `optional`** are on all three; `one` and
  `optional` reject a row count they do not admit with a `DecodeError`.
  `{ tagged: true }` gives `Value`s.
- **`stream`** on a pool or connection is an async iterator pulling one row
  per `next()`, so an unread result holds the server back. Its connection is
  freed at the last row; leaving the loop early, `return()` or `close()`
  discards it, because rows were left unread on it.
- **`signal`**: acquiring, running a statement, opening a transaction or
  savepoint and pulling a stream's rows each take an `AbortSignal`. Aborting
  rejects with the signal's reason and discards the connection the operation
  ran on — its outcome is unknown — ending any transaction on it.
- **Releasing**: a pool, a connection and a stream each have `close()` and
  work with `await using`; a transaction ends with its callback.
  Closing a connection returns it to its pool at once; closing a pool cancels
  what runs and waits until every connection is released. Nothing waits on
  garbage collection: a handle collected unclosed releases only what is idle,
  rolling back an idle transaction and returning an idle connection, and never
  cuts short work still running.

`TimeoutError` is a pool's `acquireTimeout` running out; an `AbortSignal`
timeout rejects with the signal's own `TimeoutError` `DOMException`.

### COPY

```js
import { col, copyFrom, copyTo, select, Table } from "@necessary/pgorm";

const load = copyFrom("accounts", "csv").columns(["id", "name"]).header();
await pool.copyIn(load, fs.createReadStream("accounts.csv"));        // rows written

const dump = copyTo(select(col("id"), col("name")).from(new Table("accounts")).where(col("active").eq(true)), "binary");
for await (const chunk of await pool.copyOut(dump)) output.write(chunk);   // Uint8Array chunks

await pool.copyInRows(copyFrom("accounts", "binary").columns(["id", "name"]), [[1, "Ada"], [2, null]]);
for await (const [id, name] of await pool.copyOutRows(dump)) console.log(id, name);
```

- **`copyFrom(table, format)`** and **`copyTo(source, format)`** build
  `COPY … FROM STDIN` and `COPY … TO STDOUT` in `"text"`, `"csv"` or
  `"binary"`; a COPY TO reads a table, or a `select`, or a write with a
  `returning` list, and a write without one is a `ConstructionError` when
  `copyTo` is called. The format is the builder's type: an option it does not
  take — `quote` on text, `header` on binary, `freeze` on a COPY TO — is no
  method of its type, and called anyway is a `ConstructionError`. COPY takes
  no parameters, so a value in its `where` or query is written into the SQL as
  a literal, and `inspect()` gives no values; `execute` and the other
  terminals refuse a COPY.
- **`copyIn(statement, source)`** on a pool, connection or transaction writes
  a `Uint8Array`, or an iterable or async iterable of them — a generator, a
  web `ReadableStream`, a Node.js stream of buffers — pulling the next chunk
  only as the COPY takes the last, and resolves with the rows written. A
  source that throws, or yields anything but a `Uint8Array`, abandons the
  COPY — nothing is written; inside a transaction it is aborted until rolled
  back — and the promise rejects with that error. A row the server refuses
  rejects with its `DatabaseError` once the data is sent.
- A COPY FROM the server refuses at its start — a missing table or column, a
  column list naming a generated column, an option it rejects — rejects with
  its `DatabaseError` and closes the connection, ending any transaction on it;
  a pool lends a fresh connection next time. A generated column, stored or
  virtual, takes no data, so name only the columns the rows supply: the server
  refuses a list naming one with SQLSTATE `42P10`, and pgorm does not check
  the list first, since that would cost a catalogue read before every COPY.
  With no column list a COPY leaves the generated columns out, and the server
  computes them as the rows go in; `copyInRows` describes a generated column
  without complaint, since a `select` reads one, so its COPY is refused at the
  start in the same way.
- **`copyOut(statement)`** resolves, once the server accepts the statement,
  with a `CopyOutStream` of `Uint8Array` chunks, one per `next()`; a chunk is
  a piece of the format's byte stream, not a row.
- **`copyInRows(statement, rows)`** and **`copyOutRows(statement)`** run a
  binary COPY as rows of values: `rows` is an iterable or async iterable of
  arrays, each holding a value for every column in order, read as parameters
  are and written against the column types the server describes before the
  COPY starts; a row of another width, or a value its column's type does not
  take, is a `ConstructionError` that abandons the COPY. `copyOutRows`
  resolves with a `CopyRowStream` of arrays, each value decoded as a result
  column of its type is, and `columns` naming the columns.
- A COPY holds what it runs on — a pool's takes a connection of its own; a
  connection or transaction refuses other work with a `LifecycleError` until
  the COPY ends — and, unlike `stream`, it runs in a transaction. Leaving a
  `CopyOutStream` early, or aborting a COPY with its `signal`, discards the
  connection; an aborted COPY in a transaction ends it.

### LISTEN and NOTIFY

```js
await using listener = await pool.listener({ capacity: 1024 });
await listener.listen("jobs");                                    // exact name: case and punctuation kept

await pool.transaction(async (tx) => {
  await tx.notify("jobs", JSON.stringify({ id: 42 }));            // delivered at commit
});

for await (const { channel, payload, processId } of listener) {
  console.log(channel, payload, processId);
}
```

- **`pool.listener()`** opens a `Listener` on a connection of its own, with
  the pool's configuration and TLS; it is not one the pool lends, and it
  closes with the pool. `listen`, `unlisten` and `unlistenAll` change its
  channels, and may run while a pull waits; a channel name longer than 63
  bytes is a `ConstructionError`, since PostgreSQL would listen on it
  truncated.
- The listener is an async iterator of frozen `{ channel, payload, processId
  }` notifications, one per `next()`, in the order the server sent them;
  another pull while one waits is a `LifecycleError`. Nothing reconnects it:
  a lost connection ends it with that failure after the notifications already
  received, and a notification arriving while `capacity` wait unread closes it
  with a `ConnectionError`, since one would otherwise be lost. Either way,
  re-read what the notifications were tracking and open a new listener.
- `next({ signal })` takes an `AbortSignal`, and an aborted pull leaves the
  listener open; an aborted `listen` or `unlisten` closes it, since the
  server may have acted on it. `close()`, `await using` and a `for await`
  loop leaving early close the listener and its connection; a pull waiting
  when it closes ends as done, and one waiting when its pool closes rejects
  with a `LifecycleError`.
- **`notify(channel, payload)`** on a pool, connection or transaction sends
  `SELECT pg_notify($1, $2)` with both bound; in a transaction it is
  delivered when the transaction commits, and never if it rolls back. A
  payload of 8000 bytes or more is the server's `DatabaseError`.

## Statements

pgorm-query's statements and expressions are built from JavaScript as
pgorm-python builds them, and run through the same terminals as SQL text:

```js
import { Condition, call, col, select, Table, With } from "@necessary/pgorm";

const account = new Table("account", { schema: "app", alias: "a" });
const event = new Table("event", { alias: "e" });
const clicks = call("count", event.col("id"));

const query = select(account.col("name"), clicks.as("clicks"))
  .from(account)
  .join(event, account.col("id").eq(event.col("account_id")), { kind: "left" })
  .where(Condition.any(account.col("active").eq(true), account.col("mood").isNull()))
  .groupBy(account.col("name"))
  .having(clicks.gte(1))
  .orderBy(clicks.desc({ nulls: "last" }))
  .limit(10);

query.inspect();               // { sql: 'SELECT "a"."name", COUNT("e"."id") AS ..', values: [Value, ..] }
await pool.query(query);       // built as inspect() builds it; options come second
for await (const row of pool.stream(query, { tagged: true })) { /* .. */ }
```

- **Builders never change.** Each method returns a new builder from a copy of
  its receiver's pgorm-query state, so a base query can be extended in two
  directions. Builders come from the module's functions (`select`, `col`,
  `bind`, `literal`, `call`, `caseWhen`, `caseOf`, `exists`, `scalar`, `array`, `tuple`,
  `tableFunction`), the static factories of `GroupingElement` and the
  `Table` and `With` constructors.
- **Values are bound, identifiers quoted.** An operand that is not an
  expression is a parameter, inferred as one is or declared with `Value` — an
  enum's label is cast to its type. `null` is refused (test with `isNull()`, or
  bind `Value.null(kind)`). An `Interval`, or a `Temporal.Duration`, binds as
  pgorm's interval value, its months, days and microseconds kept apart. Only
  `literal(v)` writes a value into the SQL text instead, as pgorm-query
  renders a constant, read as a parameter is and cast as its kind needs. A
  name is 1–63 bytes, always an identifier, its dots included.
- **Expressions**: comparisons and arithmetic (`eq` .. `gte`, `add` .. `mod`,
  `concat`), `and`/`or`/`not` and `Condition.all`/`any`, `isNull`, `isIn` a list
  or a `Select`, `between` (`{ symmetric }`), `like`/`ilike`/`similarTo` over
  a string or a `likeExpr(pattern)` with its `.escape(c)` or `.noEscape()`,
  `startsWith`/`endsWith`/`containsText` (text, not patterns),
  `cast`, `collate`, `at`/`slice`, `caseWhen(..).when(..).else(..)`,
  `caseOf(x).when(..)`, `call` for the functions pgorm-query constructs
  (`uuidv7` and its shift among them, the set-returning `generate_series`
  and `unnest`, whose overloads need a bound argument typed:
  `bind(1).cast("int8")`, and the record-returning `json_to_record`,
  `jsonb_to_record`, `json_to_recordset` and `jsonb_to_recordset`), `exists`,
  `scalar`, and `array(select)`, `ARRAY(select)`: the subquery's one column as
  one array value, empty for no rows, to cast and compare as a value.
- **ANY and ALL**: `any(source)` and `all(source)` are `ANY (..)` and
  `ALL (..)`, the right operand of every operator PostgreSQL reads a
  quantifier after and of nothing else: `eq` .. `gte`, `like`, `notLike`,
  `ilike` and `notIlike` (a pattern that carries no escape), the regular-expression matches,
  `contains`, `containedBy`, `overlaps`, the range positions, `matches` and
  `pathExists`. `similarTo`, `isDistinctFrom`, `between`, arithmetic and
  `concat` refuse one, and it has no `inspect` of its own. The server types a
  bound array by the operator it follows: `pathExists` and `matches` over a
  `jsonb` read one as `jsonpath[]`, so cast it,
  `bind(paths).cast("jsonpath", { array: true })`. The source is a `Select`, whose rows it ranges over,
  or an array: an expression (an array column, `array(select)`) or a value
  bound as one array parameter. `col("id").eq(any([1, 3]))` is
  `"id" = ANY($1)`, the membership `isIn` tests with one placeholder for any
  length, so the statement's text is the same at every length;
  `col("id").ne(all(ids))` is its complement, and
  `col("score").gt(all(select(..)))` compares with every row. Over an empty
  array or no rows, `ANY` is false and `ALL` true.
- **SELECT**: `select(..)`, `from`, `join` (`kind`, `lateral`), `crossJoin`,
  `where`, `groupBy` (expressions, and the grouping elements
  `GroupingElement.set`, `.empty`, `.rollup`, `.cube` and `.sets`),
  `groupByDistinct`, `having`, `orderBy`, `limit`/`offset`
  (`null` removes), `limitWithTies` (`FETCH FIRST n ROWS WITH TIES`, in
  LIMIT's slot), `distinct`, `union` .. `exceptAll` (a chain combines in
  call order: `a.union(b).intersect(c)` is `(a UNION b) INTERSECT c`),
  `lock("update", { of, wait })`, `with`, and `as(alias)` to read a query as
  a FROM item.
  `tableFunction(call)` reads a set-returning call as one, built as
  pgorm-query's `TableFunction` is — `TableFunction.rowsFrom(call)`, `.and(call)`
  for `ROWS FROM` over several, `.withOrdinality()`, `.columns(..names)`, and
  for a call returning `record` (`json_to_record` and its kin)
  `.columnDefinition("a", "integer")` for each column that types it — and
  finished by `.alias(name)`, and
  `table.tablesample("bernoulli" | "system", percentage, { repeatable })`
  samples a table. `new With(name, query, { columns, materialized }).cte(..)`
  and `With.recursive(name, query, { search, cycle })` are its common table
  expressions.
- **Refusals happen as a builder is called**: an argument of a type the method
  does not take — a number for a name, a plain value where an expression or a
  `Select` goes, a string outside a closed choice, an unknown option — throws
  a `TypeError` there, and one that cannot be built — an unknown function, a
  negative limit, a LIKE escape of two characters, an empty tuple — a
  `ConstructionError`, so a statement that cannot be built never exists. A statement binding more than 65,535 values is refused
  before anything is sent, and so is a tie limit with no ORDER BY or beside
  SKIP LOCKED — by `inspect()` and every terminal, since the order may come
  after the limit, and as soon as such a query is read into another
  statement.

### Writes

```js
import { col, Conflict, deleteFrom, insert, ReturningRow, Table, update } from "@necessary/pgorm";

const account = new Table("account", { schema: "app" });

await pool.query(
  insert(account).columns("id", "name").values(1, "Alice").values(2, "Bob")
    .onConflict(Conflict.on("id").update("name"))
    .returning([col("id"), ReturningRow.old.col("id").isNull().as("inserted")]),
);
await pool.execute(update(account).set("visits", col("visits").add(1)).where(col("id").eq(1)));
await pool.query(deleteFrom(account).where(col("active").eq(false)).returning([ReturningRow.old.star()]));
```

- **INSERT** names its distinct columns once, then takes rows: `values(..)` per
  row, checked against the columns' count, `select(query)`, or
  `defaultValues()` with no columns. `overriding("systemValue" | "userValue")`
  is for identity columns.
- **Conflicts** are an arbiter and an action: `Conflict.doNothing()` for any
  conflict, or `Conflict.on(column | expression, ..)` — with `.where(..)` for a
  partial index — or `Conflict.onConstraint(name)`, followed by `.doNothing()`,
  `.update(column, ..)` (from `EXCLUDED`) or `.set(column, value)`, an update
  taking a `.where(..)` of its own. A target without its action is refused.
- **UPDATE** assigns each column once with `set`; **DELETE** comes from
  `deleteFrom`. Each needs `where(..)` or an explicit `allRows()` before it runs,
  and `from(item)` / `using(item)` add the items it reads.
- **RETURNING**: `returning(items, { oldAs, newAs })`, every column when
  `items` is empty. `ReturningRow.old` and `ReturningRow.new` read the row
  before and after the write; a renamed version answers only to its new name,
  `col(name, { table: newName })`.
- A write with a RETURNING list is a common table expression's body, and every
  write takes `with(..)`.
- An INSERT with no rows, an UPDATE with no assignment and an UPDATE or DELETE
  with neither `where` nor `allRows()` are refused, by `inspect()` and the
  terminals alike, before anything is sent.

### MERGE

```js
import { merge, MergeAction, ReturningRow, Table } from "@necessary/pgorm";

const target = new Table("account", { alias: "t" });
const source = new Table("staged", { alias: "s" });

await pool.query(
  merge(target, source, target.col("id").eq(source.col("id")))
    .whenMatched(MergeAction.update("name", source.col("name")).set("visits", target.col("visits").add(1)))
    .whenMatched(MergeAction.delete(), { condition: source.col("name").isNull() })
    .whenNotMatched(MergeAction.insert("id", source.col("id")).set("name", source.col("name")))
    .whenNotMatchedBySource(MergeAction.delete())
    .returningAction()
    .returning([target.col("id"), ReturningRow.old.col("name").as("was")]),
);
```

`merge(target, source, on)` gives a `PendingMerge`, which has no `inspect()`
and which every terminal refuses: PostgreSQL refuses a MERGE with no WHEN arm.
Its first arm gives the `Merge`. An arm takes only what its kind of row can
take — a target row (`whenMatched`, `whenNotMatchedBySource`) is updated,
deleted or left alone, a source row (`whenNotMatched`) is inserted or skipped —
and anything else is a `ConstructionError`. `MergeAction.update` and `.insert`
take their first column at once, so neither is ever empty. Within a kind of
row, a row takes the first conditional arm whose condition holds, otherwise the
one unconditional arm, which renders last; `returningAction()` puts
`merge_action()` first in the RETURNING list, `only()` writes `ONLY`, and a
MERGE takes a plain WITH clause and is one's body when it returns rows. The
source may be a `Table` or a `FromItem`, a subquery among them.

### SQL/JSON, windows and ranges

```js
import {
  call, col, FrameType, jsonDefault, jsonExists, jsonTable, JsonTableColumn as C, jsonValue,
  Range, select, Table, Value, Window, windowFunction,
} from "@necessary/pgorm";

const docs = new Table("docs", { alias: "d" });
const size = jsonValue(docs.col("doc"), "$.size", { returning: "integer", onEmpty: jsonDefault(0), onError: "error" });
const blue = jsonExists(docs.col("doc"), "$.tags[*] ? (@ == $Tag)", { passing: { Tag: "blue" } });
const items = jsonTable(docs.col("doc"), "$.items[*]", [C.ordinality("i"), C.value("n", "integer")], { alias: "jt" });
await pool.query(select(docs.col("id"), size.as("size"), items.col("n")).from(docs).from(items).where(blue));

const running = new Window().partitionBy(col("kind")).orderBy(col("at").asc())
  .frame(FrameType.rows.preceding(1).andFollowing(1).exclude("currentRow"));
await pool.query(select(call("sum", col("weight")).over(running).as("around"), windowFunction("rank").over("w"))
  .from(new Table("reading")).window("w", new Window().orderBy(col("weight").desc())));

await pool.query(select().from(new Table("booking"))
  .where(col("seats").overlaps(new Value(new Range(1, 10, "[]"), "int4range"))));
```

- **SQL/JSON**: `jsonExists`, `jsonValue`, `jsonQuery`, `jsonObject`,
  `jsonArray`, `jsonArrayQuery`, `jsonObjectagg`, `jsonArrayagg`, `jsonParse`
  (`JSON(..)`), `jsonScalar`, `jsonSerialize`, `formatJson` and
  `isJson`/`isNotJson`. A path is bound as `jsonpath`, PASSING takes an object
  of names to values, and each function takes only its own behaviours —
  `jsonExists` `"true" | "false" | "unknown" | "error"`, `jsonValue` `"null" |
  "error" | jsonDefault(v)`, `jsonQuery` those and `"emptyArray" |
  "emptyObject"`, with one `shaping`. `jsonValue` refuses to return `json` or
  `jsonb`; `jsonQuery` reads JSON out instead. RETURNING and JSON_TABLE's
  columns take a `DataType` — `new DataType("numeric", { precision, scale })`,
  `.array()` — or a built-in type's name.
- **JSON_TABLE** is a FROM item: `jsonTable(context, path, columns, { alias,
  passing, pathName, onError })`, its columns `JsonTableColumn.ordinality`,
  `.value`, `.query`, `.exists` and `.nested`, each with its own options. It
  needs a column and an alias, and is implicitly LATERAL, so `join(items,
  bind(true), { kind: "left" })` keeps a row whose document yields nothing.
- **Windows**: `expr.over(window | extended | name)` on a function call or a
  JSON aggregate; `new Window().partitionBy(..).orderBy(..).frame(..)`; frames from
  `FrameType.rows`, `.range` or `.groups` offer only the ends that may follow
  their start; `windowFunction(name, ..)` for `row_number`, `rank`, `lag` and
  the others, usable only with `over`. Each `select.window(name, window)`
  appends an entry to the WINDOW clause, and `new ExtendedWindow(base)` builds
  one on an earlier entry, copying its partitioning and any ordering and
  adding `orderBy(..)` and `frame(..)`; given to `over`, it is written inline,
  `OVER ("base" ..)`, on any entry. A clause PostgreSQL refuses — a name
  declared twice, an extended window whose base is no earlier entry, has a
  frame, or already orders its rows when it adds an ORDER BY — an
  `over(name)` no entry declares, and an inline extended window over a base
  it could not extend as an entry are a `ConstructionError` when the
  statement is inspected, run or read into another.
- **Ranges**: `contains` (`@>`), `containedBy` (`<@`) and `overlaps` (`&&`)
  over ranges, multiranges and arrays, and `strictlyLeftOf` (`<<`),
  `strictlyRightOf` (`>>`), `notExtendRightOf` (`&<`), `notExtendLeftOf`
  (`&>`) and `adjacentTo` (`-|-`) over ranges and multiranges. A range binds
  as a `Value` naming its kind — a built-in one or a `CreatedRange` — never
  inferred, unless a model's column, whose kind is declared, takes it.
- **Regular expressions, text search and JSON paths**: `regex` (`~`),
  `notRegex` (`!~`), `iregex` (`~*`) and `notIregex` (`!~*`) match POSIX
  regular expressions, the pattern bound unless it is an expression.
  `matches` is `@@`: a `tsvector` against a `tsquery`, text against text as
  PostgreSQL's full-text conversion reads it, or a `jsonb` against a
  `jsonpath` predicate, written cast:
  `doc.matches(bind("$.a == 1").cast("jsonpath"))`. `pathExists(path)` is
  `@?`, the `jsonpath` string bound as text cast to `jsonpath`; unlike
  `jsonExists`, a GIN index on the column serves it, and a strict-mode path
  that would raise answers NULL.

## Models

A model is a table and its columns, declared in JavaScript and lowered into
the statement builders, as pgorm-python's `Model` is: declaring one sends
nothing, runs no DDL and stands for no Rust entity. Its reads decode into
records — plain objects keyed by field — and its relations, graphs, cursors
and pages follow pgorm's own `RelationDef`, `SelectGraph`, `Cursor` and
`Paginator`, down to the SQL they write.

```ts
import { column, Conflict, model, type RowOf, TypeName } from "@necessary/pgorm";

const Account = model("account", {
  schema: "app",
  columns: {
    id: column("i64", { primaryKey: true, generated: "byDefault" }),
    name: column("text"),
    displayName: column("text", { name: "display_name", nullable: true }),
    mood: column(new TypeName("mood", { schema: "app" }), { values: ["calm", "glad"], nullable: true }),
    tags: column("text", { array: true, default: true }),
  },
});
const Post = model("post", {
  schema: "app",
  columns: { id: column("i64", { primaryKey: true }), authorId: column("i64", { name: "author_id" }), title: column("text") },
});
const posts = Account.hasMany(Post, { from: "id", to: "authorId" });
const author = Post.belongsTo(Account, { from: "authorId", to: "id" });

type Account = RowOf<typeof Account>; // { id: bigint; name: string; displayName: string | null; mood: "calm" | "glad" | null; .. }

const ann = await Account.insert({ name: "Ann", mood: "calm" }).returning().one(pool);
await Account.update({ displayName: null }).where(Account.key({ id: ann.id })).execute(pool);
const { old, new: now } = await Account.update({ name: "Anne" }).where(Account.key({ id: ann.id })).returningChange(pool);
const upserted = await Account.insert({ id: 1, name: "Ann" }).onConflict(Conflict.on("id").update("name")).returningUpsert(pool);

const calm = await Account.find().where(Account.col("mood").eq("calm")).orderBy(Account.col("id").asc()).all(pool);
const authored = await posts.load(pool, calm);                         // one query: a list of posts per account
for (const [post, by] of await Post.graph().joinOne(author).all(pool)) { /* .. */ }
const page = await Account.find().cursor("name", "id").after("Ann", 1n).first(20).all(pool);
const pages = Account.find().orderBy(Account.col("id").asc()).paginate(50);
```

- **Why a factory.** `model(name, { columns })` infers everything from one
  object literal: the TypeScript declarations derive a record's type
  (`RowOf`), an insert's (`InsertOf`, requiring each field neither nullable,
  defaulted nor generated), an update's (`UpdateOf`) and a key's (`KeyOf`)
  from the columns alone, an enum's listed values becoming a union of its
  labels. A class declared with decorators or static fields would need the
  types written twice, and Node.js runs TypeScript by stripping types, which
  decorators are not.
- **Columns** take a kind every result decodes as — any value kind but `u64`
  and `char` — or a schema-qualified `TypeName`, `CreatedRange` or
  `CreatedMultirange`, and the options `name`, `nullable`, `array`,
  `primaryKey` (each such column, in order, makes the key), `default`,
  `generated` (`"always"`, which no write sets, or `"byDefault"`) and an
  enum's `values`. A declaration of the wrong shape — an unknown option, an
  option of the wrong type — throws a `TypeError` as it is made, and one that
  cannot be used a `ConstructionError`.
- **Unique keys**: `uniqueKeys: [{ fields: ["realm", "handle"], name,
  nullsNotDistinct }]` declares the table's unique keys, as an entity's
  `unique(..)` does, each of one field or several, its `name` and
  `nullsNotDistinct` optional. A key naming no field, a field the model lacks
  or one field twice is a `ConstructionError`; `describe()` reports them under
  `uniqueKeys`, and `createDisjoint` writes them into each model's table.
- **Records** are checked against the declaration as they decode: the
  result's columns are exactly the ones selected, each of its declared kind —
  an `int8` read into an `i32` field, or another schema's enum, is a
  `DecodeError` — with no NULL where the field is not nullable and no label an
  enum field does not list. The driver reports a `CreatedMultirange` column
  with no subtype to read its binary form by, so a model projects one cast to
  `text`, or an array of one to `text[]`, as pgorm's entities read it, and
  reads that text as a `Multirange` of the declared subtype: text the subtype
  does not read is a `DecodeError`.
- **Reads**: `find()`, `select(..fields)`, `findByKey(key)`; `where`,
  `orderBy`, `limit`, `offset`, and `join(relation)` to filter on a related
  row; `all`, `one`, `optional`, `count` on a pool, connection or
  transaction, and `stream` on a pool or connection. `col(field)` compares
  through the field's kind — an enum's label binds cast to its type, and a
  range or multirange field takes a `Range` or `Multirange` as it stands —
  and refuses `null`, which `isNull()` tests.
- **Writes**: a field left out stays out — the table's default on insert,
  the column unchanged on update — and one set to `null` writes NULL. An
  update or delete needs `where` or `allRows()`. An insert is typed by its
  row count, as Rust's `Insert<A, OneRow>` and `Insert<A, ManyRows>` are:
  `insert(values)` gives a `ModelInsert` and `insertMany(rows)` a
  `ModelInsertMany`, and `add(values)` or `addMany(rows)` on either gives the
  `ModelInsertMany` of its rows and theirs, its conflict clause kept.
  `returning(..fields)` reads the rows written; `returningChange(s)` an
  update's rows before and after; and an insert's `returningUpsert`, on a
  `ModelInsert`, or `returningUpserts`, on a `ModelInsertMany`, whether it
  inserted each row or updated the one its conflict found, as pgorm's
  `exec_returning_change(s)` and `exec_returning_upsert(s)` read them.
- **Relations** (`belongsTo`, `hasOne`, `hasMany`) pair fields of two
  models. `relation.find(row)` reads the far end of a record and
  `relation.load(db, rows)` the far end of many in one query; a key nothing
  matches is a `DecodeError`, never a missing row.
- **Graphs**: `graph().joinOne(relation)` is an INNER JOIN decoded as a
  record, `joinMaybe` a LEFT JOIN decoded as a record or `null`, `via` a
  junction no record reads, each source projected under `s0_`, `s1_`, .. as
  pgorm's `SelectGraph` projects it; a row is the root's record alone, or a
  tuple of every source's. `alias` names a second table of one model, `from`
  the source a relation starts at, and `allGrouped` gathers a one-slot
  graph's records under each root.
- **Cursors and pages**: `cursor(..fields)` pages by keyset — `after`,
  `before`, `first`, `last`, `asc`, `desc`, and on a graph the slots'
  primary keys as tiebreaks, with `afterWith` resuming inside a run of one
  root — and `paginate(size)` by limit and offset, with `numItems`,
  `numPages` and `pages()`. `inspect("count")`, `inspect("returningChanges")`
  and the like show what a terminal sends.

### Disjoint models

A disjoint model stores a value of one of several kinds across tables: a base
model whose every row has a kind, and a model for each kind that carries data
of its own, keyed by the base row's key. It is pgorm's `DisjointShape`, so
every statement is the one the Rust `#[derive(Disjoint)]` declaration of the
same tables sends.

```ts
const kind = new TypeName("vehicle_kind", { schema: "app" });
const Vehicle = model("vehicle", { schema: "app", columns: {
  id: column("i32", { primaryKey: true, generated: "byDefault" }),
  kind: column(kind, { values: ["car", "truck", "bike"] }),
  name: column("text"),
} });
const Car = model("car", { schema: "app", columns: { id: column("i32", { primaryKey: true }), seats: column("i16") } });
const Fleet = disjoint("vehicle", Vehicle, { car: Car, bike: null }, { schemaName: "app" });

for (const statement of createDisjoint(Fleet)) await pool.execute(statement);
const [base, kindOf, car] = await Fleet.insert({ name: "mini" }, "car", { seats: 4 }).returningModel(pool);
const roomy = await Fleet.find().filterKind("car", Car.col("seats").gte(4)).all(pool);
await Fleet.changeKind({ id: base.id }, "bike").returningModel(pool);
```

`disjoint(name, base, kinds, { kind, enumName, schemaName })` names the base's
kind field and the kind type; what the Rust declaration refuses is a
`ConstructionError` thrown as the model is declared. A read's row is
`[base record, kind, kind record or null]`, typed as a union over the kinds;
its terminals are `all`, `one` and `stream`, beside `count`, `paginate` and
`cursor`. `insert`, `update` and `changeKind` are one statement each whose
terminal `returningModel` reads back the written row; a write that touches
no row is a `DecodeError`. A registered enum is `disjointEntity(name)`, its
records the entities' own and its writes their ActiveModels (`ENTITIES.md`).

A change to the kinds once the tables exist is written from the declaration
as it will be, as pgorm's `DisjointShape` writes it: `addDisjointKind(model,
value)`, `addDisjointKindRecreating`, `removeDisjointKind`,
`addDisjointKindTable` and `removeDisjointKindTable(model, value, table)`.
Each gives `{ steps }`, each step its `statements`, to run in one transaction
before the next step, and its `gap`, `null` or `{ kind, table }`: the rows the
program writes in `table`, one for every base row of `kind`, before the next
step runs. Clear the pool's statement caches after a step that recreates the
kind type.

A change ships once, and the declaration keeps changing after it, so a
migration history freezes each change when it is made rather than computing
it again as it runs. `migrationFiles(change, name)` freezes a change as pgorm
does, a `MigrationFile` per step named `name_1`, `name_2` (or `name` for one
step), every step but the last opening with `-- pgorm: commits-after` and a
step with a gap ending with a comment naming the rows to write;
`migrationFile(name, statements, { commitsAfter })` freezes any statements,
such as `createDisjoint`'s. A file's `freeze(directory)` writes `<name>.sql`
only where no such file is, and answers whether it did, so the code that froze
each change can run again without rewriting what has shipped. Each file is one
migration for a tool that runs a directory of SQL files, each in its
transaction:

```js
for (const file of migrationFiles(addDisjointKind(Fleet, "van"), "m20260301_000001_add_van")) {
  file.freeze("migrations"); // m20260301_000001_add_van_1.sql, m20260301_000001_add_van_2.sql
}
```

`oneOf(name, model, { post: ["postId", Post], photo: ["photoId", Photo] }, {
optional })` declares a reference from a model's nullable fields to exactly one
of several targets, or at most one where `optional` says so: `createOneOf`
gives its `CHECK`, `set(value, key)` the fields a write sets (the target's to
the key, the rest `null`), `key(record)` a record's reference as `[value, key]`
or `null`, and `load(db, records)` each record's `[value, target record]`,
reading each target's table once. What a Rust `#[derive(OneOf)]` does not
compile is a `ConstructionError` as the reference is declared, and a row
setting two fields a `DecodeError`. A registered enum is `oneOfEntity(name)`,
its targets named as its variants in snake_case and its `set(active, value,
key)` writing the entity's ActiveModel.

`disjoint(name, base, kinds, { unkinded: true })` declares tables an existing
schema holds with no kind column, read by which kind's table has a row for
each base row, `null` naming at most one kind of a row with none. Nothing there
holds a base row to one kind's table, so the model is read-only: its type is
an `UnkindedDisjoint`, with no `insert`, `update` or `changeKind`, which
`createDisjoint` does not take, and each refuses with a `ConstructionError` at
run time. A base row with rows in two kinds' tables, or in none where every
kind has a model, is a `DecodeError`.

## Schema

DDL is built from JavaScript as the statements are, through pgorm-query's DDL
builders, and runs through `execute`:

```js
import {
  alterTable, col, ColumnDef, createDomain, createIndex, createSchema, createSequence, createTable,
  createType, DataType, domainValue, Table, TypeName, Value,
} from "@necessary/pgorm";

const mood = new TypeName("mood", { schema: "app" });
const seats = new TypeName("seats", { schema: "app" });
const booking = new Table("booking", { schema: "app" });

await pool.execute(createSchema("app", { ifNotExists: true }));
await pool.execute(createType(mood).values(["calm", "tense"]));
await pool.execute(createDomain(seats, "smallint").default(1).check(domainValue().between(1, 12)));
await pool.execute(
  createTable(booking)
    .column(new ColumnDef("id", "bigint").identity("always", { startWith: 1000 }))
    .column(new ColumnDef("room", "integer").notNull({ name: "room_present" }))
    .column(new ColumnDef("seats", seats))
    .column(new ColumnDef("during", "tstzrange").notNull())
    .column(new ColumnDef("mood", mood).default(new Value("calm", mood)))
    .column(new ColumnDef("price", new DataType("numeric", { precision: 8, scale: 2 })).check(col("price").gte(0)))
    .primaryKey("id")
    .unique("room", { withoutOverlaps: "during", name: "no_double_booking" })
    .foreignKey("room", new Table("room", { schema: "app" }), "id", { onDelete: "cascade" }),
);
await pool.execute(alterTable(booking).addCheck(col("room").lt(500), { name: "small", notValid: true }));
await pool.execute(createIndex(booking, { on: "during", order: "desc" }).using("gist").where(col("room").gt(0)));
await pool.execute(createSequence("ticket").options({ incrementBy: 10, maxValue: null }));
```

- **A statement renders as it runs.** PostgreSQL takes no parameters in DDL,
  so a value a statement carries — a `DEFAULT`, a `CHECK`'s operands, an enum
  label, a comment — is written as pgorm-query's escaped literal, and
  `inspect()` gives `{ sql, values: [] }`. Names are identifiers, quoted.
  Nothing runs until a terminal runs it. Every complete statement is a `DDL`
  (pgorm-python's `DDL`), a `Statement` the terminals take.
- **Schemas**: `createSchema(name, { ifNotExists })`, `dropSchema(names, {
  ifExists, behavior })` over one name or several, and `renameSchema(name,
  newName)`. A schema is named by one string, never a `Table`. A drop of a
  schema that holds anything is refused (`2BP01`) unless its behavior is
  `"cascade"`.
- **Tables**: `createTable(table)` with `column`, `primaryKey` (one; a later
  call replaces it), `unique`, `foreignKey` and `check`. A key takes `name`,
  `include`, `deferrability` and PostgreSQL 18's `withoutOverlaps` (its period
  column, written last); a unique key `nullsNotDistinct`. A foreign key pairs
  its columns with as many referenced ones and takes `onDelete`, `onUpdate`,
  `deferrability`, `enforcement` and PostgreSQL 18's `period: [column,
  referenced]`. A `ColumnDef` takes `notNull({ name, noInherit })`, `null`,
  `default`, `check`, `generated(expr, "stored" | "virtual")`,
  `identity("always" | "byDefault", sequenceOptions)`, `autoIncrement` and
  `collate`. `dropTable`, `renameTable`, `renameColumn`, `renameConstraint`
  and `truncateTable`; `commentOnTable` and `commentOnColumn`.
- **Alterations**: `alterTable(table)` has no action and so nothing to run;
  its first of `addColumn`, `modifyColumn`, `dropColumn`, `addPrimaryKey`,
  `addUnique`, `addForeignKey`, `addCheck`, `addNotNull`, `dropConstraint`,
  `validateConstraint`, `alterConstraint`, `setExpression` and
  `dropExpression` gives the statement, which takes more. `notValid` adds a
  foreign key, `CHECK` or `NOT NULL` without checking the rows already there,
  which `validateConstraint` checks later. A `modifyColumn` refuses what no
  `ALTER COLUMN` writes, a generated expression among it.
- **Indexes**: `createIndex(table, entry, { name })` over columns, expressions
  and `{ on, order, operatorClass }`, with `unique`, `nullsNotDistinct`,
  `ifNotExists`, `using(method)`, `include` and `where`; `dropIndex(table,
  name)`.
- **Types**: `createType(name)` — `.values(labels)` for an enumeration,
  `.attribute(name, type)` for a composite, `.asRange(subtype, { .. })` for a
  range — and `alterType(name)` with `addValue`, `renameTo`, `renameValue`,
  `renameAttribute` and a composite's `addAttribute`, `dropAttribute` and
  `alterAttribute`; `dropType`.
- **Domains**: `createDomain(name, base)` over any column type, with
  `collate`, `default`, `notNull` or `null` (one slot) and `check(condition,
  { name })`, a condition over `domainValue()`, PostgreSQL's `VALUE`.
  `alterDomain(name)` has no action and so nothing to run; one of
  `setDefault`, `dropDefault`, `setNotNull`, `dropNotNull`,
  `addCheck(condition, { name, notValid })`, `dropConstraint`,
  `renameConstraint`, `validateConstraint` and `renameTo` is the whole
  statement, PostgreSQL taking one action per `ALTER DOMAIN`. `dropDomain`
  over one name or several. A column of a domain is a `ColumnDef` typed
  with its `TypeName`.
- **Sequences and extensions**: `createSequence(name)` and
  `alterSequence(name)` with `asType`, `options({ incrementBy, minValue,
  maxValue, startWith, cache, cycle })` — the options an identity column
  takes, `null` a bound's `NO` form — `ownedBy` and, altering, `restart`;
  `dropSequence` and `renameSequence`; `createExtension` and `dropExtension`.
- **Views**: `createView(name, select)` with `orReplace`, `columns`,
  `checkOption("local" | "cascaded")`, `securityBarrier` and
  `securityInvoker`; `createRecursiveView(name, columns, select)`, which
  takes no check option; `createMaterializedView(name, select)` and
  `createTableAs(name, select)` with `ifNotExists`, `columns` and
  `withNoData`; `refreshMaterializedView(name, { concurrently, withNoData })`,
  never both; `dropView` and `dropMaterializedView`. A view's query is
  written with its values as literals, a definition binding nothing.
- An options object holding a key its builder does not know is a
  `TypeError`, so a misspelt option is never left out.

## Pipelines

pgorm's PRQL-shaped pipeline is the module's `pipeline` namespace:
relation-to-relation stages over sources, compiled through prqlc to
PostgreSQL SQL and run by its own terminals or as any statement is.

```js
import { pipeline as pl, Table } from "@necessary/pgorm";

const items = new Table("items", { schema: "app" });
const category = pl.col("items", "category");
const amount = pl.col("items", "amount");
const total = pl.alias("total");

const minimum = 10;
const top = pl.from(items)
  .filterWith((binder) => amount.gt(binder.bind(minimum)))   // $1, minted by this stage's binder
  .group(category)
  .aggregate(pl.sum(amount).as(total))
  .filter(total.gt(100))                                      // $2: a value is bound, not written
  .sort(total.desc())
  .take(5);                                                   // LIMIT 5: a count is no expression

top.inspect();               // { sql: "SELECT category, COALESCE(SUM(amount), 0) AS total ..", values: [..] }
await top.all(pool);         // Pipeline::all; top.one(pool) and top.optional(pool) append take 1
await pool.query(top);       // as any statement runs
```

- **Stages**: `from(source)`, then `filter`, `derive`, `select`, `group(..)`
  and `aggregate(..)`, `window(over, ..)`, `sort`, `take`, `takeRange`,
  `join(source, on, { kind })`, `append`, `intersect`, `remove` and
  `distinct`, each returning a new pipeline. A source is a `Table` (read under
  its alias if it has one), a table's name, another pipeline — embedded whole,
  its bound values with it — or `pl.source(relation).named(name)`, which is
  how a relation meets itself.
- **Expressions** are the pipeline's own, not the statement builders':
  `pl.col(table, column)` (prqlc has no catalog, so columns are qualified;
  a registered entity's column, `entity.col(name)`, is compared as the
  entity types it), `pl.alias(name)` for a name a stage introduces,
  `pl.thisColumn` and `pl.thatColumn` for a join's sides, the comparison,
  logical and arithmetic methods, `coalesce`, `isNull`, `inArray`, `cast`,
  `comparesAs(type)` — the type a column compares as, the column and each
  value beside it cast to it: the enum a domain is over, or `money`, which
  PostgreSQL needs and no catalog tells prqlc —, `as`, `asc`/`desc`,
  `pl.caseWhen`, the aggregates (`sum`, `min`, `max`, `average`, `stddev`,
  `count`, `countDistinct`, `countRows`) and the window functions
  (`rowNumber`, `rank`, `rankDense`, `lag`, `lead`, `first`, `last`) over
  `pl.over().by(..).sortBy(..).rows(start, end)`.
- **Values**: `pl.literal(v)` writes a value into the SQL; any other value an
  operand is given is bound, by the stage that takes it.
- **Binders**: each stage has a `With` form whose function is called once
  with a `Binder`; `bind(v)` mints one placeholder, reusable in the
  expressions the function returns. Like pgorm's lifetime-branded binder, it
  binds only while its function runs, and a placeholder anywhere but its own
  stage — a plain stage, another binder's, another pipeline, a window's keys —
  is a `LifecycleError`.
- **Terminals**: a pipeline's own `all(db)`, `one(db)` and `optional(db)`
  are Rust's `Pipeline::all`, `one` and `one_opt`, on a pool, a connection or
  a transaction; `one` and `optional` append `take 1`, as Rust's do, and
  `inspect("one")` shows the SQL they send. The pool's, connection's and
  transaction's terminals take a pipeline too, with their own cardinality:
  there `one` wants exactly one row. What prqlc cannot compile — an alias it
  reserves, a name it cannot write — is a `ConstructionError` before anything
  is sent.

## Registered entities

An application with pgorm entities in Rust registers them, and its graph
shapes, in a `Registry` and builds one native module carrying the binding's
API and its registrations, from a `#[neon::main]` of its own that calls
`pgorm_napi::install`. JavaScript then reaches each by name, and every
operation is the entity's own Rust API: `Select<E>` and its terminals,
`ActiveModelTrait`'s writes with the application's hooks, the terminals that
return a row's two versions, `SelectGraph` with its cursor, and pipelines
that read an entity's table and decode a registered source tuple's rows
through `select_sources`. The binding's own module registers nothing. [ENTITIES.md](ENTITIES.md) shows how to build
the module and use it; `node pgorm-napi/checks/entities.js` builds the
repository's application fixture and runs its suite in both runtimes.

```js
import { entity, graph } from "./lib/index.js";

const Account = entity("app.Account");
const busy = await Account.find().where(Account.col("mood").eq("busy")).all(pool);
const saved = await Account.active().set("id", 1).set("display name", "Ann").insert(pool);  // hooks run
const rows = await graph("app.AccountNotes").find({ aliases: ["n"] }).all(pool);        // [account, note | null][]
```

## Generated TypeScript

`pgorm-napi/codegen` builds an application's module from an
`application.json` naming its entity crate, entities, graphs and source tuples,
then writes a TypeScript module typing each registration from what the built
library describes: `AccountRecord` with each column's type, nullability and an
enum's labels, `AccountInput`, graph and source rows as tuples of records.
The generated module checks, as it loads, that the library is the one it was
written from. [CODEGEN.md](CODEGEN.md) has the workflow;
`node pgorm-napi/checks/codegen.js` runs it on the repository's fixture.

```sh
node pgorm-napi/codegen/cli.js scaffold application.json --pgorm-source . --output app-module
node pgorm-napi/codegen/cli.js build app-module
node pgorm-napi/codegen/cli.js emit app-module    # app-module/lib/app.js and app.d.ts
```

## Values

A parameter is bound, never interpolated, and each value has one JavaScript
type, both ways. Precision comes first: a value reaches JavaScript exactly or
not at all, and one that cannot is a `DecodeError` or `ConstructionError`,
never a rounded number, a truncated time or a string.

| PostgreSQL | JavaScript |
| --- | --- |
| `boolean` | `boolean` |
| `int2`, `int4`, `oid`, `"char"` | `number` |
| `int8` | `bigint`, never a number |
| `float4`, `float8` | `number` |
| `text`, `varchar`, `bpchar`, `name`, an enum | `string` |
| `bytea` | `Uint8Array` |
| `numeric` | `Decimal`, its text keeping its scale (`"19.9900"`) |
| `uuid` | `Uuid` |
| `json`, `jsonb` | JSON values; an integer past 2^53 − 1 as a `bigint` |
| `date`, `time`, `timestamp` | `Temporal.PlainDate`, `PlainTime`, `PlainDateTime` |
| `timestamptz` | `Temporal.Instant` |
| `interval` | `Interval`: months, days and microseconds, each signed |
| `inet`, `cidr`; `macaddr` | `string`; six-byte `Uint8Array` |
| arrays | arrays, `null` for a NULL item |
| ranges and multiranges | `Range`, `Multirange` |
| NULL | `null` |

Why these:

- **`numeric`** is a `Decimal`, not a number, because a float cannot hold
  `0.1` or `19.9900`. It is exact within pgorm's range (a 96-bit coefficient,
  28 fractional digits), is made from a string or a `bigint`, and refuses to
  become a number implicitly.
- **Dates and times** are Temporal's, not `Date`: a `Date` holds milliseconds
  and one meaning, an instant, where PostgreSQL has microseconds and four
  types. A `timestamptz` is an instant — PostgreSQL stores no zone with it, and
  the binary protocol sends UTC microseconds — so it reads as a
  `Temporal.Instant`, the same whatever the session's `TimeZone`, which only
  shapes its text. A `ZonedDateTime` or a `Date` parameter is refused rather
  than have its zone dropped or its meaning guessed, and a `PlainDateTime`
  binds only to `timestamp` and an `Instant` only to `timestamptz`. A value
  with a digit below the microsecond is refused; round it first, e.g.
  `instant.round({ smallestUnit: "microsecond" })`.
- **`interval`** is the module's `Interval` rather than a `Temporal.Duration`,
  whose fields must share one sign and so cannot hold `1 mon -2 days`.
  `Interval.from(duration)` and `interval.toDuration()` convert where they can.
- **`timetz`** is refused: no Temporal type is a time of day at a fixed offset.
  Select it cast to `time` or `text`.

A plain parameter's kind is inferred where its JavaScript type leaves one
answer: a safe-integer `number` binds as an integer, any other number as a
`float8`, a `bigint` as `int8`, a plain object as JSON, an array as an array of
its items' one kind. For anything else, declare the kind with `Value`, using
pgorm-python's kind names:

```js
import { Range, TypeName, Value } from "@necessary/pgorm";

new Value(5, "i16");                                  // an int2
Value.null("uuid");                                   // a typed NULL
Value.json(null);                                     // JSON's null, not SQL NULL
Value.array("i64", []);                               // an empty int8[]
new Value(new Range(1, 5), "int4range");              // a range's kind is never inferred
new Value("calm", new TypeName("mood", { schema: "app" }));
```

`{ tagged: true }` gives each column as a `Value`
carrying its kind, which tells an `int2` from an `int8` and SQL NULL from JSON's
`null`. A range type a schema created is a `CreatedRange` kind; its value
travels as text for the statement to cast, `CAST($1::text AS app.floatrange)`,
as a newtype under pgorm's own `CreatedRange` derive does. `Value.array` of a
`CreatedRange` holds each element as its text and is cast to the type's array,
`CAST($1::text[] AS app.floatrange[])`; a column of one decodes element by
element, and a query's array of a `CreatedMultirange` is a `DecodeError`, as a
column of one is.

## Errors

Every failure rejects with a subclass of `PgormError`: `DatabaseError` when
PostgreSQL refuses a statement, with its `sqlstate`, `severity` and optional
diagnostic fields (`detail`, `hint`, `schema`, `table`, `column`, `datatype`,
`constraint`); `ConnectionError` when the server cannot be reached;
`ConstructionError` for an argument pgorm cannot send, an argument of a type
the call does not take being JavaScript's own `TypeError`; `DecodeError` for a
result that does not decode; `LifecycleError` for a handle used after it
closed or while another operation holds it; `TimeoutError` when no connection
frees up in time. Credentials never appear in a message.

A constraint violation rejects with a subclass of `DatabaseError` named for
it: `UniqueViolationError` (`23505`), `ForeignKeyViolationError` (`23503`),
`CheckViolationError` (`23514`), `NotNullViolationError` (`23502`) and
`ExclusionViolationError` (`23P01`, which a `WITHOUT OVERLAPS` key raises
too). A table's constraint is named in `constraint`, a `NOT NULL` column in
`column`, and a domain whose `CHECK` or `NOT NULL` refused the value in
`datatype`.

## Tests

One suite, written against `node:test` and `node:assert`, runs unchanged in
both runtimes against a live server. It reads `PGORM_TEST_DSN`, or else the
server `DATABASE_URL` names (from the environment or the checkout's
`.env.local`/`.env`, as the Rust suite does) and its `postgres` database.

```sh
cd pgorm-napi
node --test "tests/*.test.ts"
deno test --allow-ffi --allow-read --allow-env --allow-run tests/
deno check
```

`tests/exit.test.ts` runs each of `tests/fixtures/` in a fresh process of the
runtime under test, which is why Deno's suite needs `--allow-run`, and holds it
to exiting by itself. `tests/runtime.test.ts` drives a panic on the runtime and
Neon's drop queue through two exports only debug builds carry, skipped against
a release build. `tests/values.test.ts`, `tests/connections.test.ts`, `tests/statements.test.ts`
and the other live files each make a database of their own, named for the
runtime and process, and drop it when they end.
`tests/statements-parity.test.ts` holds each statement family's cases under
`tests/parity/` to the golden file beside them, which the addon's Rust unit
tests build the same statements to with pgorm-query directly. The suite
connects in plaintext unless its connection string names an `sslmode`; with
`PGORM_TEST_CA` naming a PEM CA whose certificate for `localhost` the server
presents, it also holds verified TLS to that CA, as CI does. `deno.json` keeps
Deno on its global npm cache, so type checking `node:` imports needs no
`node_modules`.

`tests/package/` is the installed packages' smoke suite. It imports
`@necessary/pgorm` by name and runs only in the projects `checks/package.js`
installs the packed packages into ([DISTRIBUTION.md](DISTRIBUTION.md#build-and-verify)),
so `deno test` leaves it out here, while `deno check` holds it to the
declarations through the package's own name.

`tests/models.test.ts` and `tests/graphs.test.ts` make their databases
through `tests/model-fixtures.ts`. `tests/models-parity.test.ts` holds the
models' statements to `tests/parity/models.json`, which the Rust test
`models::parity` builds with pgorm's own entities — `find`, `Insert`,
`Update::many`, `Delete::many` and `SelectGraph` — and, where pgorm composes a
statement inside a terminal, with pgorm-query as the terminal does.
`tests/model-types.test.ts` is held by `deno check`: it type-checks only while
the declarations type a model's records, writes, keys, graphs and cursors from
its columns, and refuse what the binding refuses.
