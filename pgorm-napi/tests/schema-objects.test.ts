// Indexes, types, sequences and extensions built from JavaScript against a
// live server, in either runtime: what each statement makes, what the catalog
// then holds, and what the server does with it.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import {
  alterSequence,
  alterType,
  call,
  col,
  ColumnDef,
  ConstructionError,
  createExtension,
  createIndex,
  createSequence,
  createTable,
  createType,
  DatabaseError,
  DataType,
  dropExtension,
  dropIndex,
  dropSequence,
  dropType,
  Pool,
  POSTGRES_TARGET,
  renameSequence,
  Table,
  TypeName,
  Value,
} from "../lib/index.js";
import { mistyped, scratchDatabase } from "./support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

before(async () => {
  database = await scratchDatabase("pgorm_napi_schema_objects");
  shared = new Pool(database.dsn, { maxSize: 4 });
  await pool().execute("CREATE SCHEMA app");
});

after(async () => {
  await shared?.close();
  await database?.drop();
});

function refused(pattern: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
    assert.match(error.message, pattern);
    return true;
  };
}

function rejectedWith(sqlstate: string): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof DatabaseError, `expected a DatabaseError, got ${error}`);
    assert.equal(error.sqlstate, sqlstate, error.message);
    return true;
  };
}

/** The definition PostgreSQL gives the index `name`. */
async function indexdef(name: string): Promise<string | null> {
  const row = await pool().optional("SELECT indexdef FROM pg_indexes WHERE indexname = $1", [name]);
  return row ? String(row.indexdef) : null;
}

// [spec:pgorm:req:bindings.schema-indexes/test]
test("an index over expressions and columns, with its operator class, order, INCLUDE and predicate, is the one PostgreSQL holds", async () => {
  const person = new Table("person", { schema: "app" });
  await pool().execute(
    createTable(person).column(new ColumnDef("id", "integer")).column(new ColumnDef("email", "text"))
      .column(new ColumnDef("active", "boolean")).column(new ColumnDef("nick", "text")),
  );
  await pool().execute(
    createIndex(person, { on: call("lower", col("email")), operatorClass: "text_pattern_ops", order: "desc" }, {
      name: "person_email",
    })
      .column({ on: "id", order: "asc" })
      .unique()
      .include(["nick"])
      .where(col("active").eq(true)),
  );
  assert.equal(
    await indexdef("person_email"),
    "CREATE UNIQUE INDEX person_email ON app.person USING btree (lower(email) text_pattern_ops DESC, id) " +
      "INCLUDE (nick) WHERE (active = true)",
  );
  await pool().execute("INSERT INTO app.person VALUES (1, 'A@x', true, 'a'), (1, 'a@X', false, 'b')");
  await assert.rejects(pool().execute("INSERT INTO app.person VALUES (1, 'a@x', true, 'c')"), rejectedWith("23505"));
  await pool().execute(createIndex(person, "nick", { name: "person_nick" }).nullsNotDistinct().ifNotExists());
  await pool().execute(createIndex(person, "nick", { name: "person_nick" }).ifNotExists());
  await pool().execute("INSERT INTO app.person VALUES (2, 'n@x', true, NULL)");
  await assert.rejects(pool().execute("INSERT INTO app.person VALUES (3, 'm@x', true, NULL)"), rejectedWith("23505"));
  await pool().execute(createIndex(person, col("email"), { name: "person_trgm" }).using("hash"));
  assert.match(String(await indexdef("person_trgm")), /USING hash \(email\)/);
  await pool().execute(dropIndex(person, "person_nick"));
  await pool().execute(dropIndex(person, "person_nick", { ifExists: true }));
  assert.equal(await indexdef("person_nick"), null);
  await assert.rejects(pool().execute(dropIndex(person, "person_nick")), rejectedWith("42704"));
});

// [spec:pgorm:req:bindings.schema-types/test]
test("an enumeration's labels are data, in the order created, added and renamed", async () => {
  const mood = new TypeName("Mood \"name\"", { schema: "app" });
  await pool().execute(createType(mood).values(["calm", "O'Brien", "", "back\\slash"]));
  await pool().execute(alterType(mood).addValue("tense", { before: "calm" }));
  await pool().execute(alterType(mood).addValue("last"));
  await pool().execute(alterType(mood).addValue("middle", { after: "O'Brien" }));
  await pool().execute(alterType(mood).renameValue("calm", "serene"));
  const labels = await pool().query(
    `SELECT enumlabel FROM pg_enum WHERE enumtypid = 'app."Mood ""name"""'::regtype ORDER BY enumsortorder`,
  );
  assert.deepStrictEqual(labels.map((row) => row.enumlabel), ["tense", "serene", "O'Brien", "middle", "", "back\\slash", "last"]);
  await pool().execute(createTable("feeling").column(new ColumnDef("mood", mood).default(new Value("middle", mood))));
  await pool().execute("INSERT INTO feeling DEFAULT VALUES");
  assert.equal((await pool().one(`SELECT mood::text AS m FROM feeling`)).m, "middle");
  await assert.rejects(pool().execute(dropType(mood)), rejectedWith("2BP01"));
  await pool().execute(alterType(mood).renameTo("feeling_kind"));
  await pool().execute(dropType(new TypeName("feeling_kind", { schema: "app" }), { behavior: "cascade" }));
  assert.deepStrictEqual(Object.keys(await pool().one("SELECT * FROM feeling")), []);
  await pool().execute(dropType(["nothing_here", "nor_here"], { ifExists: true }));
});

// [spec:pgorm:req:bindings.schema-types/test]
test("a composite's attributes are created, added, retyped, dropped and renamed, CASCADE reaching a typed table", async () => {
  await pool().execute(
    createType(new TypeName("pair", { schema: "app" })).attribute("a", "integer")
      .attribute("b", "text", { collation: { name: "C", schema: "pg_catalog" } }),
  );
  await pool().execute("CREATE TABLE app.pairs OF app.pair");
  const pair = new TypeName("pair", { schema: "app" });
  await assert.rejects(pool().execute(alterType(pair).addAttribute("c", "bigint")), rejectedWith("2BP01"));
  await pool().execute(
    alterType(pair).addAttribute("c", new DataType("varchar", { length: 5 })).alterAttribute("a", "bigint")
      .dropAttribute("b").dropAttribute("missing", { ifExists: true }).behavior("cascade"),
  );
  await pool().execute(alterType(pair).renameAttribute("c", "label").behavior("cascade"));
  const attributes = await pool().query(
    `SELECT attname, format_type(atttypid, atttypmod) AS type FROM pg_attribute
     WHERE attrelid = 'app.pair'::regclass AND attnum > 0 AND NOT attisdropped ORDER BY attnum`,
  );
  assert.deepStrictEqual(attributes.map((row) => [row.attname, row.type]), [["a", "bigint"], ["label", "character varying(5)"]]);
  assert.deepStrictEqual(
    (await pool().query("SELECT column_name FROM information_schema.columns WHERE table_name = 'pairs' ORDER BY ordinal_position"))
      .map((row) => row.column_name),
    ["a", "label"],
  );
  await pool().execute("INSERT INTO app.pairs VALUES (1, 'x')");
  assert.equal((await pool().one("SELECT (ROW(2, 'y')::app.pair).label AS l")).l, "y");
});

// [spec:pgorm:req:bindings.schema-types/test]
test("a created range type is a range over its subtype, with the multirange it is given", async () => {
  await pool().execute(
    createType(new TypeName("floatrange", { schema: "app" })).asRange("double", {
      subtypeDiff: "float8mi",
      multirangeTypeName: new TypeName("floats", { schema: "app" }),
    }),
  );
  const range = await pool().one(
    `SELECT format_type(rngsubtype, NULL) AS subtype, rngsubdiff::text AS diff, rngmultitypid::regtype::text AS multi
     FROM pg_range WHERE rngtypid = 'app.floatrange'::regtype`,
  );
  assert.deepStrictEqual([range.subtype, range.diff, range.multi], ["double precision", "float8mi", "app.floats"]);
  const contained = await pool().one(
    "SELECT app.floatrange(1.5, 2.5) @> 2.0::float8 AS hit, app.floats(app.floatrange(1, 2))::text AS m",
  );
  assert.deepStrictEqual([contained.hit, contained.m], [true, "{[1,2)}"]);
  await pool().execute(createType("bare"));
  assert.equal((await pool().one("SELECT typtype::text AS t FROM pg_type WHERE typname = 'bare'")).t, "p");
  await pool().execute(dropType([new TypeName("floatrange", { schema: "app" }), "bare"]));
});

// [spec:pgorm:req:bindings.schema-sequences/test]
test("a sequence hands out the values its options say, restarts, and goes with the column that owns it", async () => {
  const ticket = new Table("ticket", { schema: "app" });
  await pool().execute(
    createSequence(ticket).asType("smallint").options({ startWith: 10, incrementBy: -3, minValue: 1, cycle: true })
      .options({ maxValue: 10 }),
  );
  const next = async (name = "app.ticket") =>
    (await pool().one("SELECT nextval($1::text::regclass) AS n", [name])).n;
  assert.deepStrictEqual([await next(), await next(), await next(), await next(), await next()], [10n, 7n, 4n, 1n, 10n]);
  await pool().execute(alterSequence(ticket).restart(4).options({ incrementBy: 1, cycle: false }));
  assert.deepStrictEqual([await next(), await next()], [4n, 5n]);
  await pool().execute(alterSequence(ticket).restart());
  assert.equal(await next(), 10n);
  await assert.rejects(next(), rejectedWith("2200H"));
  const bounds = await pool().one("SELECT data_type, maximum_value FROM information_schema.sequences WHERE sequence_name = 'ticket'");
  assert.deepStrictEqual([bounds.data_type, bounds.maximum_value], ["smallint", "10"]);
  await assert.rejects(pool().execute(alterSequence(ticket).options({ incrementBy: 0 })), rejectedWith("22023"));
  await pool().execute("CREATE TABLE app.counter (id bigint)");
  await pool().execute(alterSequence(ticket).ownedBy(new Table("counter", { schema: "app" }), "id").ifExists());
  await pool().execute(renameSequence(ticket, "voucher"));
  await pool().execute("DROP TABLE app.counter");
  assert.equal(await pool().optional("SELECT 1 FROM pg_class WHERE relname = 'voucher'"), null);
  await pool().execute(createSequence("spare").ifNotExists().options({ maxValue: 9_007_199_254_740_993n }));
  await pool().execute(createSequence("spare").ifNotExists());
  assert.equal((await pool().one("SELECT seqmax FROM pg_sequence WHERE seqrelid = 'spare'::regclass")).seqmax, 9_007_199_254_740_993n);
  await pool().execute(dropSequence(["spare", "absent"], { ifExists: true }));
  await assert.rejects(pool().execute(dropSequence("spare")), rejectedWith("42P01"));
});

// [spec:pgorm:req:bindings.schema-sequences/test]
test("an identity column's sequence takes the options a sequence does", async () => {
  await pool().execute(
    createTable("numbered").column(
      new ColumnDef("n", "bigint").identity("byDefault", { startWith: 100, incrementBy: 10, cache: 1 }),
    ).column(new ColumnDef("x", "integer")),
  );
  await pool().execute("INSERT INTO numbered (x) VALUES (1), (2)");
  await pool().execute("INSERT INTO numbered (n, x) VALUES (5, 3)");
  assert.deepStrictEqual((await pool().query("SELECT n FROM numbered ORDER BY x")).map((row) => row.n), [100n, 110n, 5n]);
});

// [spec:pgorm:req:bindings.schema-sequences/test]
// [spec:pgorm:req:sql.target+2/test]    btree_gist 1.7 on 18, and on 19, which ships
// 1.9 alone, the refusal of 1.7
test("an extension is created in the schema and at the version asked, and dropped", async () => {
  let version = "1.7";
  if (POSTGRES_TARGET === 19) {
    await assert.rejects(
      pool().execute(createExtension("btree_gist", { schema: "app", version: "1.7" })),
      rejectedWith("22023"),
    );
    version = "1.9";
  }
  await pool().execute(createExtension("btree_gist", { schema: "app", version }));
  await pool().execute(createExtension("btree_gist", { ifNotExists: true }));
  const installed = await pool().one(
    "SELECT extversion, extnamespace::regnamespace::text AS schema FROM pg_extension WHERE extname = 'btree_gist'",
  );
  assert.deepStrictEqual([installed.extversion, installed.schema], [version, "app"]);
  await assert.rejects(pool().execute(createExtension("btree_gist")), rejectedWith("42710"));
  await pool().execute(dropExtension("btree_gist", { behavior: "cascade" }));
  await pool().execute(dropExtension("btree_gist", { ifExists: true }));
  await assert.rejects(pool().execute(createExtension("no_such_extension")), rejectedWith("0A000"));
});

// [spec:pgorm:req:bindings.schema-types/test]
// [spec:pgorm:req:bindings.schema-sequences/test]
test("labels, types and sequence clauses PostgreSQL cannot take are refused as they are built", async () => {
  assert.throws(() => createType("t").values(["x".repeat(64)]), refused(/63 UTF-8 bytes/));
  assert.throws(() => createType("t").values(["a\0b"]), refused(/NUL/));
  assert.throws(() => createType("t").values("abc" as never), TypeError);
  assert.throws(() => alterType("t").addValue("x", { before: "a", after: "b" } as never), refused(/not both/));
  assert.throws(() => createType("t").asRange("double", { canonical: "f" } as never), TypeError);
  assert.throws(() => createType(42 as never), mistyped(/string or a TypeName/));
  assert.throws(() => alterSequence("s").options({}), refused(/at least one option/));
  assert.throws(() => alterSequence("s").options({ cache: 2n ** 63n }), refused(/cache/));
  assert.throws(() => createSequence("s").asType("numeric" as never), mistyped(/"smallint", "integer", "bigint"/));
  assert.throws(() => createSequence("s").ownedBy(undefined as never, "id"), mistyped(/string or a Table/));
  assert.throws(() => createIndex("t", { on: "a", order: "up" } as never), mistyped(/order/));
  assert.throws(() => createIndex("t", { on: "a", nulls: "last" } as never), TypeError);
  assert.throws(() => createIndex("t", new Table("x") as never), mistyped(/column's name or an expression/));
  assert.throws(() => createExtension("e", { version: "1\0" }), refused(/NUL/));
  const pending = alterSequence("s");
  assert.equal("inspect" in pending, false);
  await assert.rejects(pool().execute(pending as never), refused(/ALTER SEQUENCE needs a clause/));
  await assert.rejects(pool().execute(alterType("t") as never), refused(/ALTER TYPE needs a change/));
});
