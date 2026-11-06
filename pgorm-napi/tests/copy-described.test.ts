// A COPY's columns described without starting it, the sink a COPY of rows
// is written through a row at a time, a COPY's rows read as a model's
// records, and the binary format's encoder and decoder over columns a
// program names, against a live server in either runtime.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import {
  column,
  ConstructionError,
  CopyColumn,
  copyFrom,
  CopyRowDecoder,
  CopyRowEncoder,
  CopyRowSink,
  copyTo,
  DatabaseError,
  Decimal,
  DecodeError,
  LifecycleError,
  model,
  Pool,
  select,
  Table,
  TypeName,
} from "../lib/index.js";
import { same, scratchDatabase } from "./support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

before(async () => {
  database = await scratchDatabase("pgorm_napi_copy_described");
  shared = new Pool(database.dsn, { maxSize: 4 });
  for (
    const statement of [
      "CREATE SCHEMA moods",
      "CREATE TYPE moods.mood AS ENUM ('calm', 'loud')",
      "CREATE TABLE item (id int PRIMARY KEY, name varchar, mood moods.mood, tags text[], price money)",
      "CREATE TABLE copied (id int PRIMARY KEY, name varchar, mood moods.mood, tags text[], price money)",
    ]
  ) {
    await pool().execute(statement);
  }
});

after(async () => {
  await shared?.close();
  await database?.drop();
});

const Item = model("item", {
  columns: {
    id: column("i32", { primaryKey: true }),
    name: column("text", { nullable: true }),
    mood: column(new TypeName("mood", { schema: "moods" }), { nullable: true, values: ["calm", "loud"] }),
    tags: column("text", { array: true, nullable: true }),
  },
});

const items = new Table("item");

/** Each column as its name, its type's qualified name and its money places. */
function described(columns: readonly CopyColumn[]): [string, string, number | null][] {
  return columns.map((each) => [each.name, `${each.type.schema}.${each.type.name}`, each.moneyPlaces]);
}

async function count(table: string): Promise<number> {
  return (await pool().one(`SELECT count(*)::int AS n FROM ${table}`)).n as number;
}

function refusedWith(pattern: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
    assert.match(error.message, pattern);
    return true;
  };
}

// [spec:pgorm:req:bindings.copy-rows+2/test]    a COPY's columns are described in any format, an enum's type qualified by its schema and money's places learned
test("copyInColumns and copyOutColumns describe a COPY's columns and types without starting it", async () => {
  same(described(await pool().copyInColumns(copyFrom("item", "binary"))), [
    ["id", "pg_catalog.int4", null],
    ["name", "pg_catalog.varchar", null],
    ["mood", "moods.mood", null],
    ["tags", "pg_catalog._text", null],
    ["price", "pg_catalog.money", 2],
  ]);
  same(described(await pool().copyInColumns(copyFrom("item", "csv").columns(["tags", "id"]))), [
    ["tags", "pg_catalog._text", null],
    ["id", "pg_catalog.int4", null],
  ]);
  const query = select(items.col("mood"), items.col("id").as("key")).from(items);
  same(described(await pool().copyOutColumns(copyTo(query, "text"))), [
    ["mood", "moods.mood", null],
    ["key", "pg_catalog.int4", null],
  ]);
  const columns = await pool().copyOutColumns(copyTo("item", "binary"));
  assert.ok(columns[0]?.type instanceof TypeName);
  assert.ok(columns[0]?.equals(new CopyColumn("id", "int4")));
  assert.ok(!columns[0]?.equals(new CopyColumn("id", "int8")));
  assert.equal(await count("item"), 0);
  await assert.rejects(pool().copyInColumns(copyTo("item", "text") as never), TypeError);
  await assert.rejects(pool().copyOutColumns(copyFrom("item", "text") as never), TypeError);
});

// [spec:pgorm:req:bindings.copy-rows+2/test]    a refused description is the server's DatabaseError and leaves the connection usable
test("a description the server refuses rejects with its DatabaseError and keeps the connection", async () => {
  await using connection = await pool().acquire();
  await assert.rejects(connection.copyInColumns(copyFrom("missing", "text")), (error: unknown) => {
    assert.ok(error instanceof DatabaseError);
    assert.equal(error.sqlstate, "42P01");
    return true;
  });
  await assert.rejects(connection.copyOutColumns(copyTo("missing", "binary")), DatabaseError);
  assert.equal((await connection.one("SELECT 1 AS one")).one, 1);
  same(described(await connection.copyInColumns(copyFrom("item", "text").columns(["id"]))), [
    ["id", "pg_catalog.int4", null],
  ]);
});

// [spec:pgorm:req:bindings.copy-rows+2/test]    a transaction and a savepoint describe a COPY, and a refused description fails the transaction as a statement does
test("a transaction and its savepoint describe a COPY, holding the transaction while they do", async () => {
  const rolledBack = new Error("roll the table back");
  const outcome = pool().transaction(async (tx) => {
    await tx.execute("CREATE TABLE fleeting (at date)");
    same(described(await tx.copyOutColumns(copyTo("fleeting", "csv"))), [["at", "pg_catalog.date", null]]);
    await tx.transaction(async (savepoint) => {
      same(described(await savepoint.copyInColumns(copyFrom("fleeting", "binary"))), [["at", "pg_catalog.date", null]]);
    });
    await assert.rejects(
      tx.transaction(async (savepoint) => {
        await savepoint.copyInColumns(copyFrom("missing", "binary"));
      }),
      DatabaseError,
    );
    assert.equal((await tx.one("SELECT count(*)::int AS n FROM fleeting")).n, 0);
    throw rolledBack;
  });
  await assert.rejects(outcome, rolledBack);
  await assert.rejects(pool().copyInColumns(copyFrom("fleeting", "binary")), DatabaseError);
});

// [spec:pgorm:req:bindings.copy-rows+2/test]    a sink writes a row at a time on a pool, a connection and a transaction, and finish gives the count
test("copyInRows with no rows gives a sink that writes each row sent and counts them at its finish", async () => {
  const copy = copyFrom("item", "binary").columns(["id", "name", "mood"]);
  const sink = await pool().copyInRows(copy);
  assert.ok(sink instanceof CopyRowSink);
  same(sink.columns.map((each) => each.name), ["id", "name", "mood"]);
  for (let id = 1; id <= 300; id++) await sink.send([id, `item ${id}`, id % 2 === 0 ? "calm" : "loud"]);
  assert.equal(sink.closed, false);
  assert.equal(await sink.finish(), 300);
  assert.equal(sink.closed, true);
  await assert.rejects(sink.send([301, "late", null]), LifecycleError);
  assert.equal(await count("item"), 300);

  await using connection = await pool().acquire();
  const held = await connection.copyInRows(copyFrom("item", "binary").columns(["id"]));
  await assert.rejects(connection.one("SELECT 1 AS one"), LifecycleError);
  await Promise.all([held.send([301]), held.send([302]), held.send([303])]);
  assert.equal(await held.finish(), 3);
  assert.equal((await connection.one("SELECT count(*)::int AS n FROM item")).n, 303);

  await connection.transaction(async (tx) => {
    const inner = await tx.copyInRows(copyFrom("item", "binary").columns(["id"]));
    await inner.send([304]);
    assert.equal(await inner.finish(), 1);
    assert.equal((await tx.one("SELECT max(id) AS id FROM item")).id, 304);
  });
  await pool().execute("TRUNCATE item");
});

// [spec:pgorm:req:bindings.copy-rows+2/test]    a row the columns refuse is a ConstructionError, nothing of it written, and the sink goes on
// [spec:pgorm:req:napi.typing+1/test]
test("a sink refuses a row of the wrong width or type and goes on taking rows", async () => {
  const sink = await pool().copyInRows(copyFrom("item", "binary").columns(["id", "name"]));
  await sink.send([1, "one"]);
  await assert.rejects(sink.send([2]), refusedWith(/1 values for the COPY's 2 columns \(id, name\)/));
  await assert.rejects(sink.send(["two", "two"]), refusedWith(/COPY column `id` of type `int4`/));
  await assert.rejects(sink.send([2, new Decimal("2.5")]), refusedWith(/COPY column `name` of type `varchar`/));
  // @ts-expect-error a row is an array of its columns' values
  await assert.rejects(sink.send("two"), TypeError);
  await sink.send([2, null]);
  assert.equal(await sink.finish(), 2);
  same(
    (await pool().query("SELECT id, name FROM item ORDER BY id")).map((row) => [row.id, row.name]),
    [[1, "one"], [2, null]],
  );
  await pool().execute("TRUNCATE item");
});

// [spec:pgorm:req:bindings.copy-rows+2/test]    a sink closed, disposed or aborted before its finish writes nothing and frees its connection
test("a sink left unfinished abandons its COPY, so nothing is written", async () => {
  const copy = copyFrom("item", "binary").columns(["id"]);
  const sink = await pool().copyInRows(copy);
  await sink.send([1]);
  await sink.close();
  assert.equal(sink.closed, true);
  await assert.rejects(sink.finish(), LifecycleError);

  await using connection = await pool().acquire();
  {
    await using disposed = await connection.copyInRows(copy);
    await disposed.send([2]);
  }
  assert.equal((await connection.one("SELECT count(*)::int AS n FROM item")).n, 0);

  const controller = new AbortController();
  const aborted = await connection.copyInRows(copy, undefined, { signal: controller.signal });
  await aborted.send([3]);
  controller.abort(new Error("the upload was cancelled"));
  await assert.rejects(aborted.finish(), /the upload was cancelled/);
  assert.equal(await count("item"), 0);
});

// [spec:pgorm:req:bindings.copy-rows+2/test]    a model's records are read out of a COPY and written back through sendModel, each column from the field whose column it is
test("models() reads a COPY's rows as records, and sendModel writes records back", async () => {
  const sink = await pool().copyInRows(copyFrom("item", "binary").columns(["id", "name", "mood", "tags"]));
  await sink.sendModel({ id: 1, name: "one", mood: "calm", tags: ["a", "b"] }, Item);
  await sink.sendModel({ id: 2, name: null, mood: null, tags: null }, Item);
  await sink.sendModel({ id: 3, name: "three", mood: "loud", tags: ["c"] });
  await sink.sendModel({ id: 4, name: "four", mood: "calm", tags: [] }, Item);
  // @ts-expect-error a field takes its declared values
  await assert.rejects(sink.sendModel({ id: 5, name: "five", mood: "glum", tags: null }, Item), ConstructionError);
  await assert.rejects(sink.sendModel({ id: 5, name: "five" }), refusedWith(/no "mood"/));
  assert.equal(await sink.finish(), 4);

  const stream = await pool().copyOutRows(copyTo(select(items.star()).from(items).orderBy(items.col("id").asc()), "binary"));
  const records = [];
  for await (const record of stream.models(Item)) records.push(record);
  same(records, [
    { id: 1, name: "one", mood: "calm", tags: ["a", "b"] },
    { id: 2, name: null, mood: null, tags: null },
    { id: 3, name: "three", mood: "loud", tags: ["c"] },
    { id: 4, name: "four", mood: "calm", tags: [] },
  ]);
  assert.equal(stream.closed, true);

  const into = await pool().copyInRows(copyFrom("copied", "binary").columns(["id", "name", "mood", "tags"]));
  for (const record of records) await into.sendModel({ ...record, id: record.id + 10 }, Item);
  assert.equal(await into.finish(), 4);
  same(
    (await pool().query("SELECT id, mood::text AS mood, tags FROM copied ORDER BY id")).map((row) => [
      row.id,
      row.mood,
      row.tags,
    ]),
    [[11, "calm", ["a", "b"]], [12, null, null], [13, "loud", ["c"]], [14, "calm", []]],
  );

  const priced = await pool().copyInRows(copyFrom("copied", "binary"));
  const unpriced = { id: 20, name: null, mood: null, tags: null };
  await assert.rejects(priced.sendModel(unpriced, Item), refusedWith(/no field of item reads the COPY's column "price"/));
  await priced.close();
  await pool().execute("TRUNCATE item, copied");
});

// [spec:pgorm:req:bindings.copy-rows+2/test]    a COPY lacking a model's column, or holding a column of another kind, is a DecodeError that closes the stream
test("models() refuses a COPY lacking a field's column or holding it in another kind", async () => {
  await pool().execute("INSERT INTO item (id, name) VALUES (1, 'one')");
  const narrow = await pool().copyOutRows(copyTo(select(items.col("id"), items.col("name")).from(items), "binary"));
  await assert.rejects(narrow.models(Item).next(), (error: unknown) => {
    assert.ok(error instanceof DecodeError);
    assert.match(error.message, /no column "mood" for mood/);
    return true;
  });
  assert.equal(narrow.closed, true);

  const Wrong = model("item", { columns: { id: column("i64", { primaryKey: true }) } });
  const wrong = await pool().copyOutRows(copyTo("item", "binary"));
  await assert.rejects(wrong.models(Wrong).next(), /column "id" is i32, and id declares i64/);
  assert.equal(wrong.closed, true);
  await pool().execute("TRUNCATE item");
});

/** Every split of `data` into two chunks, at each byte. */
function* splits(data: Uint8Array): Generator<Uint8Array[]> {
  for (let at = 0; at <= data.length; at++) yield [data.subarray(0, at), data.subarray(at)];
}

function concatenated(chunks: readonly Uint8Array[]): Uint8Array {
  const data = new Uint8Array(chunks.reduce((length, chunk) => length + chunk.length, 0));
  let at = 0;
  for (const chunk of chunks) {
    data.set(chunk, at);
    at += chunk.length;
  }
  return data;
}

// [spec:pgorm:req:bindings.copy-rows+2/test]    the encoder's bytes are a COPY's data, and the decoder reads a COPY's data split at any byte
test("CopyRowEncoder writes the bytes copyIn takes and CopyRowDecoder reads copyOut's, split anywhere", async () => {
  const columns = [new CopyColumn("id", "int4"), new CopyColumn("name", "varchar"), new CopyColumn("tags", "text[]")];
  const encoder = new CopyRowEncoder(columns);
  same(encoder.columns.map((each) => each.name), ["id", "name", "tags"]);
  const data = concatenated([
    CopyRowEncoder.header(),
    encoder.encode([1, "Noon", ["a"]]),
    encoder.encode([2, null, null]),
    CopyRowEncoder.trailer(),
  ]);
  assert.throws(() => encoder.encode([3, "short"]), refusedWith(/2 values for the COPY's 3 columns/));
  assert.throws(() => encoder.encode(["three", "x", null]), refusedWith(/COPY column `id` of type `int4`/));
  // @ts-expect-error a row is an array of its columns' values
  assert.throws(() => encoder.encode("row"), TypeError);
  assert.equal(await pool().copyIn(copyFrom("item", "binary").columns(["id", "name", "tags"]), data), 2);

  const chunks: Uint8Array[] = [];
  const query = select(items.col("id"), items.col("name"), items.col("tags")).from(items).orderBy(items.col("id").asc());
  for await (const chunk of await pool().copyOut(copyTo(query, "binary"))) chunks.push(chunk);
  const out = concatenated(chunks);
  for (const split of splits(out)) {
    const decoder = new CopyRowDecoder(columns);
    const rows = [];
    for (const chunk of split) {
      decoder.push(chunk);
      for (let row = decoder.nextRow(); row !== null; row = decoder.nextRow()) rows.push(row);
    }
    decoder.finish();
    same(rows, [[1, "Noon", ["a"]], [2, null, null]]);
  }
  await pool().execute("TRUNCATE item");
});

// [spec:pgorm:req:bindings.copy-rows+2/test]    the decoder refuses data that ends part way through a row or before the trailer, and data that is not the format
test("CopyRowDecoder refuses data that ends early or is not the binary format", () => {
  const columns = [new CopyColumn("id", "int4")];
  const encoder = new CopyRowEncoder(columns);
  const row = encoder.encode([7]);
  const midRow = new CopyRowDecoder(columns);
  midRow.push(CopyRowEncoder.header());
  midRow.push(row.subarray(0, row.length - 1));
  assert.equal(midRow.nextRow(), null);
  assert.throws(() => midRow.finish(), DecodeError);

  const untrailed = new CopyRowDecoder(columns);
  untrailed.push(concatenated([CopyRowEncoder.header(), row]));
  same(untrailed.nextRow(), [7]);
  assert.equal(untrailed.nextRow(), null);
  assert.throws(() => untrailed.finish(), /before its trailer/);

  const foreign = new CopyRowDecoder(columns);
  foreign.push(new TextEncoder().encode("id\n7\n"));
  assert.throws(() => foreign.nextRow(), DecodeError);
  // @ts-expect-error the data is pushed as Uint8Array chunks
  assert.throws(() => foreign.push("id"), TypeError);
});

// [spec:pgorm:req:bindings.copy-rows+2/test]    a program names a built-in type by pg_type's name, an element's name and [], or a TypeName in pg_catalog, and nothing else
// [spec:pgorm:req:napi.typing+1/test]
test("a CopyColumn a program makes is of a built-in type, and money takes its places", () => {
  same(described([new CopyColumn("tags", "_text"), new CopyColumn("tags", "text[]")]), [
    ["tags", "pg_catalog._text", null],
    ["tags", "pg_catalog._text", null],
  ]);
  const price = new CopyColumn("price", new TypeName("money", { schema: "pg_catalog" }), { moneyPlaces: 3 });
  same(described([price]), [["price", "pg_catalog.money", 3]]);
  assert.ok(new CopyColumn("n", new TypeName("int8")).equals(new CopyColumn("n", "int8")));
  assert.throws(() => new CopyColumn("n", "integer"), refusedWith(/integer is no built-in PostgreSQL type; describe/));
  assert.throws(() => new CopyColumn("m", new TypeName("mood", { schema: "moods" })), refusedWith(/moods\.mood is no built-in/));
  assert.throws(() => new CopyColumn("p", "money"), refusedWith(/a money column needs moneyPlaces/));
  assert.throws(() => new CopyColumn("t", "text", { moneyPlaces: 2 }), refusedWith(/moneyPlaces is for a money column/));
  assert.throws(() => new CopyColumn("p", "money", { moneyPlaces: -1 }), TypeError);
  // @ts-expect-error a column names its type
  assert.throws(() => new CopyColumn("n"), TypeError);
  // @ts-expect-error a column's options are its moneyPlaces alone
  assert.throws(() => new CopyColumn("n", "int4", { places: 2 }), TypeError);
  // @ts-expect-error an encoder is made over CopyColumns
  assert.throws(() => new CopyRowEncoder(["id"]), TypeError);
});
