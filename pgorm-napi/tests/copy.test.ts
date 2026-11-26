// COPY FROM STDIN and COPY TO STDOUT against a live server, in either runtime:
// the builders' refusals, each format both ways, sources of every shape, the
// holds a COPY takes on what it runs on, and how a COPY that fails, is
// abandoned or is aborted leaves its connection and transaction.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import {
  call,
  col,
  ConstructionError,
  copyFrom,
  copyTo,
  DatabaseError,
  deleteFrom,
  insert,
  LifecycleError,
  merge,
  MergeAction,
  Pool,
  select,
  Table,
  type Transaction,
  update,
  Window,
} from "../lib/index.js";
import { holdTransaction, same, scratchDatabase } from "./support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function scratch(): string {
  if (!database) throw new Error("the scratch database was not created");
  return database.dsn;
}

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

before(async () => {
  database = await scratchDatabase("pgorm_napi_copy");
  shared = new Pool(scratch(), { maxSize: 4 });
  for (const table of ["items", "items_copy"]) {
    await pool().execute(
      `CREATE TABLE ${table} (id int PRIMARY KEY, note text, amount numeric, seen timestamptz, data bytea)`,
    );
  }
  // Enough rows that a COPY TO of them is still sending when it is left.
  await pool().execute("CREATE TABLE many AS SELECT n, repeat('z', 100) AS padding FROM generate_series(1, 50000) AS n");
});

after(async () => {
  await shared?.close();
  await database?.drop();
});

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function bytes(text: string): Uint8Array {
  return encoder.encode(text);
}

async function reset(...tables: string[]): Promise<void> {
  await pool().execute(`TRUNCATE ${tables.join(", ")}`);
}

async function count(table: string): Promise<number> {
  return (await pool().one(`SELECT count(*)::int AS n FROM ${table}`)).n as number;
}

/** Every chunk of a COPY TO's stream, joined. */
async function drained(stream: AsyncIterable<Uint8Array>): Promise<Uint8Array> {
  const chunks: Uint8Array[] = [];
  for await (const chunk of stream) chunks.push(chunk);
  const joined = new Uint8Array(chunks.reduce((total, chunk) => total + chunk.length, 0));
  let at = 0;
  for (const chunk of chunks) {
    joined.set(chunk, at);
    at += chunk.length;
  }
  return joined;
}

/** `pending`, or a rejection once `ms` pass: a refusal never turns into a wait. */
function within<T>(pending: Promise<T>, ms = 3000): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const deadline = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`still waiting after ${ms} ms`)), ms);
  });
  return Promise.race([pending, deadline]).finally(() => clearTimeout(timer));
}

function lifecycle(pattern?: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof LifecycleError, `expected a LifecycleError, got ${error}`);
    if (pattern) assert.match(error.message, pattern);
    return true;
  };
}

function sqlstate(code: string): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof DatabaseError, `expected a DatabaseError, got ${error}`);
    assert.equal(error.sqlstate, code);
    return true;
  };
}

/** A gate a source waits on, opened by the test. */
function gate(): { wait: Promise<void>; open(): void } {
  let open!: () => void;
  const wait = new Promise<void>((resolve) => (open = resolve));
  return { wait, open };
}

// [spec:pgorm:req:bindings.copy+1/test]    an option outside the statement's format or direction is refused by its type and as it is called
// [spec:pgorm:req:napi.typing+1/test]
test("an option a COPY's format or direction does not take is a ConstructionError as it is called", () => {
  const text = copyFrom("items", "text");
  const binary = copyFrom("items", "binary");
  const csvOut = copyTo("items", "csv");
  assert.throws(
    // @ts-expect-error QUOTE is the csv format's
    () => text.quote("'"),
    (error: unknown) => {
      assert.ok(error instanceof ConstructionError);
      assert.match(error.message, /QUOTE belongs to the csv format: this COPY is text/);
      return true;
    },
  );
  // @ts-expect-error HEADER is the delimited formats'
  assert.throws(() => binary.header(), /HEADER belongs to the text and csv formats/);
  // @ts-expect-error NULL is the delimited formats'
  assert.throws(() => binary.nullString(""), ConstructionError);
  // @ts-expect-error FREEZE is COPY FROM's
  assert.throws(() => csvOut.freeze(), /FREEZE belongs to COPY FROM/);
  // @ts-expect-error FORCE_NULL reads a field, so it is COPY FROM's
  assert.throws(() => csvOut.forceNull(["note"]), ConstructionError);
  // @ts-expect-error FORCE_QUOTE is COPY TO's
  assert.throws(() => copyFrom("items", "csv").forceQuoteAll(), /FORCE_QUOTE belongs to COPY TO/);
  assert.throws(() => copyFrom("items", "csv").delimiter("ab"), /one-character string/);
  assert.throws(() => copyFrom("items", "csv").quote(""), ConstructionError);
  assert.throws(() => copyFrom("items", "text").onErrorIgnore({ rejectLimit: 0 }), ConstructionError);
  // @ts-expect-error a log verbosity PostgreSQL does not name
  assert.throws(() => copyFrom("items", "text").onErrorIgnore({ logVerbosity: "loud" }), TypeError);
  // @ts-expect-error a misspelt option is refused, never left out
  assert.throws(() => copyFrom("items", "text").onErrorIgnore({ limit: 1 }), TypeError);
  // @ts-expect-error JSON is no COPY format
  assert.throws(() => copyFrom("items", "json"), TypeError, /format is one of "text", "csv", "binary"/);
  assert.throws(() => copyFrom(new Table("items", { alias: "i" }), "csv"), ConstructionError);
  // @ts-expect-error an expression is no COPY source
  assert.throws(() => copyTo(col("id"), "csv"), TypeError);
  // The same builders, used as their types allow, are untouched by a refused call.
  assert.equal(
    text.delimiter("|").inspect().sql,
    `COPY "items" FROM STDIN WITH (FORMAT text, DELIMITER '|')`,
  );
  assert.equal(text.inspect().sql, `COPY "items" FROM STDIN WITH (FORMAT text)`);
  assert.deepEqual(binary.inspect().values, []);
  assert.equal(binary.format, "binary");
});

// [spec:pgorm:req:bindings.copy+1/test]    a statement terminal refuses a COPY, and copyIn and copyOut refuse what is no COPY
test("a COPY runs through copyIn and copyOut alone, and a source of the wrong shape is a TypeError", async () => {
  await assert.rejects(pool().execute(copyFrom("items", "text")), TypeError);
  await assert.rejects(pool().query(copyTo("items", "text")), /runs through copyIn\(\.\.\) or copyOut\(\.\.\)/);
  const loose = pool();
  await assert.rejects(loose.copyIn(copyTo("items", "text") as never, bytes("")), TypeError);
  await assert.rejects(loose.copyOut(copyFrom("items", "text") as never), TypeError);
  await assert.rejects(loose.copyIn(copyFrom("items", "text"), "1\ta\n" as never), TypeError);
  await assert.rejects(loose.copyIn(copyFrom("items", "text"), 42 as never), TypeError);
  await assert.rejects(loose.copyIn(copyFrom("items", "text"), ["1\ta\n"] as never), TypeError);
  assert.equal(await count("items"), 0);
});

// [spec:pgorm:req:bindings.copy+1/test]    text and csv each round-trip through copyIn and copyOut, NULL included
test("text and csv data written by copyIn reads back through copyOut", async () => {
  await reset("items");
  const text = "1\tplain\t1.50\t\\N\t\\\\x6869\n2\t\\N\t\\N\t2026-01-02 03:04:05+00\t\\N\n";
  assert.equal(await pool().copyIn(copyFrom("items", "text"), bytes(text)), 2);
  await pool().transaction(async (tx) => {
    await tx.execute("SET LOCAL TIME ZONE 'UTC'");
    const out = await tx.copyOut(copyTo(select(col("id"), col("note"), col("amount"), col("seen"), col("data")).from(new Table("items")).orderBy(col("id").asc()), "text"));
    assert.equal(decoder.decode(await drained(out)), text);
  });
  await reset("items");
  const csv = 'id,note\n3,"with, comma"\n4,\n5,"say ""hi"""\n';
  const written = await pool().copyIn(copyFrom("items", "csv").columns(["id", "note"]).header(), [bytes(csv)]);
  assert.equal(written, 3);
  same(await pool().query("SELECT id, note FROM items ORDER BY id"), [
    { id: 3, note: "with, comma" },
    { id: 4, note: null },
    { id: 5, note: 'say "hi"' },
  ]);
  const out = await pool().copyOut(copyTo(select(col("id"), col("note")).from(new Table("items")), "csv").header().forceQuote(["note"]));
  assert.equal(decoder.decode(await drained(out)), 'id,note\n3,"with, comma"\n4,\n5,"say ""hi"""\n');
});

// [spec:pgorm:req:bindings.copy+1/test]    binary copied out of one table copies into another exactly
test("binary data copied out of one table copies into another", async () => {
  await reset("items", "items_copy");
  await pool().execute(
    "INSERT INTO items VALUES (1, 'a', 12.5, '2026-03-04T05:06:07Z', '\\x00ff'), (2, NULL, NULL, NULL, NULL), (3, 'ü', -0.001, '1999-12-31T23:59:59.999999Z', '')",
  );
  const binary = await drained(await pool().copyOut(copyTo("items", "binary")));
  assert.deepEqual([...binary.subarray(0, 11)], [...bytes("PGCOPY\n\xff\r\n\0").subarray(0, 7), 0xff, 0x0d, 0x0a, 0]);
  assert.equal(await pool().copyIn(copyFrom("items_copy", "binary"), binary), 3);
  const read = "SELECT id, note, amount::text AS amount, seen, data FROM %s ORDER BY id";
  same(await pool().query(read.replace("%s", "items_copy")), await pool().query(read.replace("%s", "items")));
});

// [spec:pgorm:req:bindings.copy+1/test]    a column subset writes those columns and the rest take their defaults
test("a COPY naming a subset of columns leaves the rest to their defaults", async () => {
  await pool().execute(
    "CREATE TABLE defaults (id int PRIMARY KEY, note text NOT NULL DEFAULT 'none', made int DEFAULT 7, marked text)",
  );
  const copy = copyFrom("defaults", "csv").columns(["id"]).columns(["marked"]);
  assert.equal(copy.inspect().sql, `COPY "defaults" ("id", "marked") FROM STDIN WITH (FORMAT csv)`);
  assert.equal(await pool().copyIn(copy, [bytes("1,x\n"), bytes("2,")], {}), 2);
  same(await pool().query("SELECT * FROM defaults ORDER BY id"), [
    { id: 1, note: "none", made: 7, marked: "x" },
    { id: 2, note: "none", made: 7, marked: null },
  ]);
  const defaulted = copyFrom("defaults", "text").columns(["id", "note"]).defaultString("@d");
  assert.equal(await pool().copyIn(defaulted, bytes("3\t@d\n")), 1);
  assert.equal((await pool().one("SELECT note FROM defaults WHERE id = 3")).note, "none");
});

// [spec:pgorm:req:bindings.copy+1/test]    COPY TO over a select inlines its WHERE value as a literal
test("a COPY TO over a select reads only the rows its WHERE value picks", async () => {
  await reset("items");
  await pool().execute("INSERT INTO items (id, note) VALUES (1, 'o''k'), (2, 'other'), (3, 'o''k')");
  const query = select(col("id")).from(new Table("items")).where(col("note").eq("o'k")).orderBy(col("id").asc());
  const copy = copyTo(query, "csv");
  assert.equal(
    copy.inspect().sql,
    `COPY (SELECT "id" FROM "items" WHERE "note" = E'o\\'k' ORDER BY "id" ASC) TO STDOUT WITH (FORMAT csv)`,
  );
  assert.deepEqual(copy.inspect().values, []);
  assert.equal(decoder.decode(await drained(await pool().copyOut(copy))), "1\n3\n");
});

// [spec:pgorm:req:bindings.copy+1/test]    each write is a COPY TO source through its RETURNING list, and refused as one without it
test("a COPY TO reads a write's RETURNING list, and a write without one is a ConstructionError", async () => {
  await reset("items", "items_copy");
  await pool().execute("INSERT INTO items (id, note) VALUES (1, 'one'), (2, 'two')");
  await pool().execute("INSERT INTO items_copy (id, note) VALUES (5, 'five')");
  const items = new Table("items");
  const mirror = new Table("items_copy");
  const read = async (source: Parameters<typeof copyTo>[0], format: "text" | "csv") =>
    decoder.decode(await drained(await pool().copyOut(copyTo(source, format))));

  assert.equal(await read(insert(items).columns("id", "note").values(9, "new").returning([col("id")]), "text"), "9\n");
  const renamed = update(items).set("note", "uno").where(col("id").eq(1)).returning([col("id"), col("note")]);
  assert.equal(await read(renamed, "csv"), "1,uno\n");
  assert.equal(await read(deleteFrom(items).where(col("id").eq(2)).returning([col("note")]), "text"), "two\n");
  const merged = merge(items, mirror, items.col("id").eq(mirror.col("id")))
    .whenNotMatched(MergeAction.insert("id", mirror.col("id")).set("note", mirror.col("note")))
    .returningAction();
  assert.equal(await read(merged, "text"), "INSERT\n");

  const silent = [
    [insert(items).columns("id").values(10), "INSERT"],
    [update(items).set("note", "x").allRows(), "UPDATE"],
    [deleteFrom(items).allRows(), "DELETE"],
    [merge(items, mirror, items.col("id").eq(mirror.col("id"))).whenMatched(MergeAction.delete()), "MERGE"],
  ] as const;
  for (const [write, keyword] of silent) {
    for (const format of ["text", "csv", "binary"] as const) {
      assert.throws(
        () => copyTo(write, format),
        (error: unknown) => {
          assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
          assert.match(error.message, new RegExp(`this ${keyword} has no RETURNING list`));
          return true;
        },
      );
    }
  }
  const rows = await pool().query("SELECT id, note FROM items ORDER BY id");
  assert.deepEqual(rows.map((row) => [row.id, row.note]), [[1, "uno"], [5, "five"], [9, "new"]]);
});

// [spec:pgorm:req:bindings.copy+1/test]    a Select source's refused tie limit or WINDOW clause is a ConstructionError when copyTo reads it
test("copyTo refuses a Select whose tie limit or WINDOW clause PostgreSQL refuses, and copies it once fixed", async () => {
  await reset("items");
  await pool().execute("INSERT INTO items (id, amount) VALUES (1, 3), (2, 3), (3, 4)");
  const items = new Table("items");
  const tied = select(items.col("id")).from(items).limitWithTies(1);
  const skipping = tied.orderBy(items.col("amount").asc()).lock("update", { wait: "skipLocked" });
  const windowed = select(call("count", items.col("id")).over("w")).from(items);
  const cases = [
    [tied, /needs an ORDER BY/],
    [skipping, /SKIP LOCKED/],
    [windowed, /no window "w" is declared/],
  ] as const;
  for (const [query, pattern] of cases) {
    for (const format of ["text", "csv", "binary"] as const) {
      assert.throws(
        () => copyTo(query, format),
        (error: unknown) => {
          assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
          assert.match(error.message, pattern);
          return true;
        },
      );
    }
  }
  const ordered = tied.orderBy(items.col("amount").asc());
  const ids = decoder.decode(await drained(await pool().copyOut(copyTo(ordered, "text"))));
  // Both rows tie on the one key, so their order is the server's.
  assert.deepEqual(ids.split("\n").sort(), ["", "1", "2"]);
  const declared = windowed.window("w", new Window());
  assert.equal(decoder.decode(await drained(await pool().copyOut(copyTo(declared, "csv")))), "3\n3\n3\n");
});

// [spec:pgorm:req:bindings.copy+1/test]    a ReadableStream, a sync iterable and an async generator each feed copyIn
test("copyIn reads a ReadableStream, an iterable and an async generator", async () => {
  await reset("items");
  const rows = ["1\ta\n", "2\tb\n", "3\tc\n"];
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const row of rows) controller.enqueue(bytes(row));
      controller.close();
    },
  });
  assert.equal(await pool().copyIn(copyFrom("items", "text").columns(["id", "note"]), stream), 3);
  async function* generated() {
    yield bytes("4\td\n5");
    yield new Uint8Array(0);
    yield bytes("\te\n");
  }
  assert.equal(await pool().copyIn(copyFrom("items", "text").columns(["id", "note"]), generated()), 2);
  assert.equal(await pool().copyIn(copyFrom("items", "text").columns(["id", "note"]), new Set([bytes("6\tf\n")])), 1);
  assert.equal(await count("items"), 6);
});

// [spec:pgorm:req:bindings.copy+1/test]    eight megabytes stream in from a generator and out chunk by chunk
test("a large source streams in, and its COPY TO is read chunk by chunk", async () => {
  await pool().execute("CREATE TABLE big (id int, padding text)");
  const padding = "x".repeat(40);
  const rows = 200_000;
  let produced = 0;
  async function* source() {
    for (let start = 0; start < rows; start += 1000) {
      let text = "";
      for (let id = start; id < start + 1000; id += 1) text += `${id}\t${padding}\n`;
      const chunk = bytes(text);
      produced += chunk.length;
      yield chunk;
    }
  }
  assert.equal(await pool().copyIn(copyFrom("big", "text"), source()), rows);
  assert.ok(produced >= 8 * 1024 * 1024, `the source made ${produced} bytes`);
  const sums = await pool().one("SELECT count(*)::int AS n, sum(id)::int8 AS total FROM big");
  same(sums, { n: rows, total: BigInt((rows * (rows - 1)) / 2) });
  let read = 0;
  let chunks = 0;
  await using connection = await pool().acquire();
  for await (const chunk of await connection.copyOut(copyTo("big", "text"))) {
    assert.ok(chunk instanceof Uint8Array);
    read += chunk.length;
    chunks += 1;
  }
  assert.equal(read, produced);
  assert.ok(chunks > 8, `${chunks} chunks`);
  assert.equal((await connection.one("SELECT 1 AS n")).n, 1);
  let whole = "";
  for (let id = rows; id < rows + 50_000; id += 1) whole += `${id}\t${padding}\n`;
  assert.equal(await connection.copyIn(copyFrom("big", "text"), bytes(whole)), 50_000);
  assert.equal((await connection.one("SELECT count(DISTINCT id)::int AS n FROM big")).n, rows + 50_000);
});

// [spec:pgorm:req:bindings.copy+1/test]    a source is pulled only as the server takes the data, never run ahead of it
test("copyIn pulls its source only as the server takes the data", async () => {
  await pool().execute("CREATE TABLE slow (id int, padding text)");
  await pool().execute(
    "CREATE FUNCTION slow_row() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(0.001); RETURN NEW; END $$",
  );
  await pool().execute("CREATE TRIGGER slow BEFORE INSERT ON slow FOR EACH ROW EXECUTE FUNCTION slow_row()");
  const chunk = bytes(Array.from({ length: 64 }, (_, id) => `${id}\t${"y".repeat(1000)}\n`).join(""));
  let produced = 0;
  function* endless() {
    for (;;) {
      produced += chunk.length;
      yield chunk;
    }
  }
  await using single = new Pool(scratch(), { maxSize: 1 });
  const controller = new AbortController();
  const reason = new Error("enough");
  const copying = single.copyIn(copyFrom("slow", "text"), endless(), { signal: controller.signal });
  const aborted = assert.rejects(copying, (error: unknown) => error === reason);
  await new Promise((resolve) => setTimeout(resolve, 1500));
  // The server takes about a megabyte a second here. What runs ahead of it is
  // the binding's queue and the two sockets' buffers, which the kernel sizes
  // at a few megabytes; a source pulled without that bound would run
  // gigabytes ahead in the time the test waits.
  assert.ok(produced < 32 * 1024 * 1024, `the source ran ${produced} bytes ahead of the server`);
  controller.abort(reason);
  await aborted;
  assert.equal(await count("slow"), 0);
  assert.equal((await single.one("SELECT 2 AS n")).n, 2);
  // The server works through the data already in its socket before it sees
  // the connection gone, holding the table meanwhile, so the test ends that
  // backend itself.
  await pool().execute(
    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = current_database() AND query LIKE 'COPY \"slow\"%'",
  );
});

// [spec:pgorm:req:bindings.copy+1/test]    a source that throws aborts the COPY, rejects with its own error and leaves the connection usable
test("a source that throws abandons the COPY, writes nothing and leaves the connection usable", async () => {
  await reset("items");
  await using connection = await pool().acquire();
  const failure = new Error("the source failed");
  async function* failing() {
    yield bytes("1\ta\n2\tb\n");
    throw failure;
  }
  await assert.rejects(connection.copyIn(copyFrom("items", "text").columns(["id", "note"]), failing()), (error: unknown) => error === failure);
  assert.equal(connection.closed, false);
  assert.equal((await within(connection.one("SELECT count(*)::int AS n FROM items"))).n, 0);
  let closed = false;
  async function* wrong() {
    try {
      yield bytes("3\tc\n");
      yield "4\td\n" as unknown as Uint8Array;
    } finally {
      closed = true;
    }
  }
  await assert.rejects(connection.copyIn(copyFrom("items", "text").columns(["id", "note"]), wrong()), (error: unknown) => {
    assert.ok(error instanceof TypeError);
    assert.match(error.message, /Uint8Array chunks, not string/);
    return true;
  });
  assert.ok(closed, "the source's iterator was closed");
  assert.equal(await connection.copyIn(copyFrom("items", "text").columns(["id", "note"]), bytes("5\te\n")), 1);
  assert.deepEqual((await connection.query("SELECT id FROM items")).map((row) => row.id), [5]);
});

// [spec:pgorm:req:bindings.copy+1/test]    a unique violation mid-copy in a transaction rejects 23505 and leaves it aborted until rolled back
test("a row the server refuses rejects with its SQLSTATE, and aborts the transaction it ran in", async () => {
  await reset("items");
  await using connection = await pool().acquire();
  async function* duplicated() {
    yield bytes("1\ta\n2\tb\n");
    yield bytes("1\tagain\n3\tc\n");
  }
  await assert.rejects(
    connection.transaction(async (tx) => {
      await tx.execute("INSERT INTO items (id) VALUES (100)");
      await assert.rejects(tx.copyIn(copyFrom("items", "text").columns(["id", "note"]), duplicated()), sqlstate("23505"));
      await tx.query("SELECT 1");
    }),
    sqlstate("25P02"),
  );
  assert.equal(connection.closed, false);
  assert.equal((await connection.one("SELECT count(*)::int AS n FROM items")).n, 0);
  await connection.transaction(async (kept) => {
    assert.equal(await kept.copyIn(copyFrom("items", "csv").columns(["id"]), bytes("7\n8\n")), 2);
    assert.equal(decoder.decode(await drained(await kept.copyOut(copyTo(select(col("id")).from(new Table("items")).orderBy(col("id").asc()), "text")))), "7\n8\n");
    await kept.transaction(async (savepoint) => {
      assert.equal(await savepoint.copyIn(copyFrom("items", "csv").columns(["id"]), bytes("9\n")), 1);
    });
  });
  assert.equal(await count("items"), 3);
});

// [spec:pgorm:req:bindings.copy+1/test]    a COPY FROM the server refuses at the start rejects 42P01 and its connection is discarded
test("a COPY FROM refused before its data flows rejects with the refusal, discarding its connection", async () => {
  await using single = new Pool(scratch(), { maxSize: 1, acquireTimeout: 2000 });
  let pulled = false;
  async function* source() {
    pulled = true;
    yield bytes("1\n");
  }
  await assert.rejects(single.copyIn(copyFrom("missing", "text"), source()), sqlstate("42P01"));
  assert.equal(pulled, false, "nothing was pulled from the source of a COPY that never started");
  assert.equal((await single.one("SELECT 3 AS n")).n, 3);
  const connection = await single.acquire();
  await assert.rejects(connection.copyIn(copyFrom("items", "text").columns(["nope"]), bytes("1\n")), sqlstate("42703"));
  assert.equal(connection.closed, true);
  await connection.close();
  const refused = await single.acquire();
  let tx = undefined as Transaction | undefined;
  await assert.rejects(
    refused.transaction(async (opened) => {
      tx = opened;
      await opened.copyIn(copyFrom("missing", "csv"), bytes("1\n"));
    }),
    sqlstate("42P01"),
  );
  assert.equal(tx?.closed, true);
  await refused.close();
  assert.equal((await single.one("SELECT 4 AS n")).n, 4);
});

// [spec:pgorm:req:bindings.copy+1/test]    a connection or transaction holding a COPY refuses other work with a LifecycleError
test("a connection and a transaction refuse other work while a COPY holds them", async () => {
  await reset("items");
  await using connection = await pool().acquire();
  const held = gate();
  async function* gated() {
    yield bytes("1\ta\n");
    await held.wait;
    yield bytes("2\tb\n");
  }
  const copying = connection.copyIn(copyFrom("items", "text").columns(["id", "note"]), gated());
  await assert.rejects(within(connection.query("SELECT 1")), lifecycle(/busy/));
  await assert.rejects(within(connection.copyOut(copyTo("items", "text"))), lifecycle(/busy/));
  await assert.rejects(within(connection.transaction(async () => {})), lifecycle(/busy/));
  held.open();
  assert.equal(await copying, 2);
  const out = await connection.copyOut(copyTo(select(col("id")).from(new Table("items")).orderBy(col("id").asc()), "csv"));
  await assert.rejects(within(connection.execute("SELECT 1")), lifecycle(/busy/));
  const first = out.next();
  await assert.rejects(out.next(), lifecycle(/one chunk is pulled at a time/));
  assert.equal((await first).done, false);
  await drained(out);
  assert.equal(out.closed, true);
  assert.equal((await connection.one("SELECT 5 AS n")).n, 5);
  await connection.transaction(async (tx) => {
    const stream = await tx.copyOut(copyTo("items", "text"));
    await assert.rejects(within(tx.query("SELECT 1")), lifecycle(/busy/));
    await assert.rejects(within(tx.transaction(async () => {})), lifecycle(/busy/));
    await drained(stream);
    assert.equal((await tx.one("SELECT 6 AS n")).n, 6);
  });
});

// [spec:pgorm:req:bindings.copy+1/test]    a pool's COPY takes a connection of its own, returned at the end and discarded if closed early
test("a pool's COPY runs on a connection of its own, returned at its end and discarded if left early", async () => {
  await reset("items");
  await using single = new Pool(scratch(), { maxSize: 1, acquireTimeout: 2000 });
  assert.equal(await single.copyIn(copyFrom("items", "csv").columns(["id"]), bytes("1\n2\n3\n")), 3);
  assert.deepEqual(single.status(), { maxSize: 1, size: 1, available: 1, waiting: 0 });
  assert.equal(decoder.decode(await drained(await single.copyOut(copyTo(select(col("id")).from(new Table("items")).orderBy(col("id").asc()), "text")))), "1\n2\n3\n");
  assert.deepEqual(single.status(), { maxSize: 1, size: 1, available: 1, waiting: 0 });
  const early = await single.copyOut(copyTo("many", "text"));
  for await (const chunk of early) {
    assert.ok(chunk.length > 0);
    break;
  }
  assert.equal(early.closed, true);
  assert.equal(single.status().size, 0, "the connection left mid-COPY was discarded");
  assert.equal((await single.one("SELECT 7 AS n")).n, 7);
  await using connection = await single.acquire();
  const stream = await connection.copyOut(copyTo("many", "binary"));
  await stream.next();
  await stream.close();
  assert.equal(connection.closed, true);
});

// [spec:pgorm:req:bindings.copy+1/test]    an abort mid-COPY rejects with its reason, discards the connection and commits nothing
test("aborting a COPY rejects with the signal's reason, discards its connection and commits nothing", async () => {
  await reset("items");
  const held = gate();
  async function* slow() {
    yield bytes("1\ta\n");
    await held.wait;
    yield bytes("2\tb\n");
  }
  const connection = await pool().acquire();
  const controller = new AbortController();
  const reason = new Error("stop copying");
  setTimeout(() => controller.abort(reason), 100);
  await assert.rejects(
    connection.copyIn(copyFrom("items", "text").columns(["id", "note"]), slow(), { signal: controller.signal }),
    (error: unknown) => error === reason,
  );
  assert.equal(connection.closed, true);
  await connection.close();

  const writing = await pool().acquire();
  let tx = undefined as Transaction | undefined;
  await assert.rejects(
    writing.transaction(async (opened) => {
      tx = opened;
      await opened.execute("INSERT INTO items (id) VALUES (50)");
      await opened.copyIn(copyFrom("items", "text").columns(["id", "note"]), slow(), { signal: AbortSignal.timeout(100) });
    }),
    (error: unknown) => error instanceof DOMException && error.name === "TimeoutError",
  );
  assert.ok(tx);
  assert.equal(tx.closed, true);
  assert.equal(writing.closed, true);
  await assert.rejects(tx.query("SELECT 1"), lifecycle(/closed/));
  await writing.close();
  held.open();

  const reader = await pool().acquire();
  await assert.rejects(
    reader.transaction(async (reading) => {
      await reading.execute("INSERT INTO items (id) VALUES (60)");
      const stream = await reading.copyOut(copyTo("many", "text"), { signal: AbortSignal.timeout(100) });
      await assert.rejects(
        (async () => {
          for await (const _ of stream) await new Promise((resolve) => setTimeout(resolve, 20));
        })(),
        (error: unknown) => error instanceof DOMException && error.name === "TimeoutError",
      );
      assert.equal(stream.closed, true);
      assert.equal(reading.closed, true);
    }),
    lifecycle(/closed/),
  );
  await reader.close();
  assert.equal(await count("items"), 0);
});

// [spec:pgorm:req:bindings.copy+1/test]    closing the connection or pool under a COPY ends it without waiting on its source or reader
// [spec:pgorm:req:bindings.cancellation/test]
test("closing what a COPY runs on ends the COPY at once, its source still waiting", async () => {
  await reset("items");
  const held = gate();
  async function* waiting() {
    yield bytes("1\ta\n");
    await held.wait;
    yield bytes("2\tb\n");
  }
  const connection = await pool().acquire();
  const copying = connection.copyIn(copyFrom("items", "text").columns(["id", "note"]), waiting());
  // The check is attached before the close, so the rejection the close causes
  // is never unhandled while the close is awaited.
  const ended = assert.rejects(within(copying), lifecycle(/connection closed during the COPY/));
  await new Promise((resolve) => setTimeout(resolve, 50));
  const started = performance.now();
  await connection.close();
  await ended;
  assert.ok(performance.now() - started < 2000);
  held.open();
  assert.equal(await count("items"), 0);

  const closing = new Pool(scratch(), { maxSize: 2 });
  const stream = await closing.copyOut(copyTo("many", "text"));
  await stream.next();
  const holding = await holdTransaction(async (use) => await (await closing.acquire()).transaction(use));
  const out = await holding.transaction.copyOut(copyTo("many", "csv"));
  await out.next();
  await closing.close();
  await assert.rejects(within(stream.next()), lifecycle(/pool closed during the COPY/));
  assert.equal(stream.closed, true);
  await assert.rejects(within(out.next()), LifecycleError);
  assert.equal(out.closed, true);
  assert.equal(holding.transaction.closed, true);
  await assert.rejects(within(holding.end()), lifecycle(/closed/));
});
