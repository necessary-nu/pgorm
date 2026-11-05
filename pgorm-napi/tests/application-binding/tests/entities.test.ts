// The application's registered entities, through its own native module,
// against a live server in either runtime: reads through `Select<E>`, writes
// through `ActiveModelTrait` with the application's hooks, the terminals
// that return a row's two versions, and registered graphs and their cursors.
// [spec:pgorm:req:napi.application+2/test]

import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import {
  all,
  any,
  bind,
  CheckViolationError,
  col as statementCol,
  Conflict,
  ConstructionError,
  CreatedRange,
  DatabaseError,
  Decimal,
  DecodeError,
  entities,
  entity,
  type EntityRecord,
  graph,
  graphs,
  LifecycleError,
  pipeline as pl,
  Pool,
  Range,
  select,
  Table,
  TypeName,
  Value,
} from "../../../lib/index.js";
import { scratchDatabase } from "../../support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

const Account = entity("app.Account");
const Note = entity("app.Note");
const Membership = entity("app.Membership");
const Parcel = entity("app.Parcel");
const Till = entity("app.Till");
const Booking = entity("app.Booking");

before(async () => {
  database = await scratchDatabase("pgorm_napi_entities");
  shared = new Pool(database.dsn, { maxSize: 4 });
  for (
    const sql of [
      "CREATE SCHEMA napi_entities",
      "CREATE TYPE napi_entities.mood AS ENUM ('calm', 'busy')",
      `CREATE TABLE napi_entities.accounts (
         id int4 PRIMARY KEY, "display name" text NOT NULL, note text, version int4 NOT NULL,
         mood napi_entities.mood NOT NULL
       )`,
      "CREATE TABLE napi_entities.notes (id int4 PRIMARY KEY, account_id int4 NOT NULL, body text NOT NULL)",
      `CREATE TABLE napi_entities.memberships (
         account_id int4, team text, role text NOT NULL, PRIMARY KEY (account_id, team)
       )`,
      "CREATE DOMAIN napi_entities.quantity AS int4 DEFAULT 1 NOT NULL CONSTRAINT positive CHECK (VALUE > 0)",
      "CREATE DOMAIN napi_entities.temper AS napi_entities.mood",
      "CREATE DOMAIN napi_entities.code AS text CHECK (VALUE <> '')",
      `CREATE TABLE napi_entities.parcels (
         id int4 PRIMARY KEY, qty napi_entities.quantity, mood napi_entities.temper NOT NULL,
         codes napi_entities.code[] NOT NULL
       )`,
      "CREATE TABLE napi_entities.tills (id int4 PRIMARY KEY, takings money NOT NULL, floats money[])",
      "CREATE TYPE napi_entities.floatrange AS RANGE (SUBTYPE = float8)",
      "CREATE TABLE napi_entities.bookings (id int4 PRIMARY KEY, span napi_entities.floatrange NOT NULL, spares napi_entities.floatrange[])",
    ]
  ) {
    await pool().execute(sql);
  }
});

beforeEach(async () => {
  await pool().execute(
    "TRUNCATE napi_entities.accounts, napi_entities.notes, napi_entities.memberships, napi_entities.parcels, napi_entities.tills, napi_entities.bookings",
  );
});

after(async () => {
  await shared?.close();
  await database?.drop();
});

async function seed(): Promise<void> {
  await pool().execute(
    `INSERT INTO napi_entities.accounts VALUES (1, 'Ann', NULL, 1, 'calm'), (2, 'Bob', 'b', 1, 'busy'),
       (3, 'Cy', NULL, 1, 'calm')`,
  );
  await pool().execute("INSERT INTO napi_entities.notes VALUES (10, 1, 'a1'), (11, 1, 'a2'), (12, 2, 'b1')");
}

function refused(pattern: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
    assert.match(error.message, pattern);
    return true;
  };
}

// [spec:pgorm:req:bindings.entities+5/test]
// [spec:pgorm:req:bindings.relation-conditions/test]
test("the module names its registrations and describes each, and refuses a name it does not register", () => {
  assert.deepStrictEqual(entities(), [
    "app.Account",
    "app.Auction",
    "app.Booking",
    "app.Label",
    "app.Listing",
    "app.Membership",
    "app.Note",
    "app.Packet",
    "app.Pallet",
    "app.Parcel",
    "app.Sale",
    "app.Shipment",
    "app.Temper",
    "app.Till",
  ]);
  assert.deepStrictEqual(graphs(), ["app.AccountNotes", "app.AccountOnly", "app.MembershipOnly", "app.MixedNotes", "app.RequiredNotes"]);
  assert.throws(() => entity("app.Missing"), refused(/no entity is registered as "app.Missing"/));
  assert.throws(() => graph("app.Missing"), refused(/no graph is registered as "app.Missing"/));
  assert.deepStrictEqual(Account.columns, ["id", "display name", "note", "version", "mood"]);
  const described = Account.describe();
  assert.equal(described.name, "app.Account");
  assert.equal(described.schema, "napi_entities");
  assert.equal(described.table, "accounts");
  assert.deepStrictEqual(described.primaryKey, ["id"]);
  assert.deepStrictEqual(described.uniqueKeys, []);
  // [spec:pgorm:req:bindings.entities+5/test]    a registration's describe() reports the unique keys its entity's unique(..) declares, each its name, its columns by their SQL names in key order and nullsNotDistinct
  assert.deepStrictEqual(entity("app.Label").describe().uniqueKeys, [
    { name: "labels_text_packet", columns: ["text", "packet_id"], nullsNotDistinct: true },
  ]);
  assert.deepStrictEqual(
    described.columns.map((column) => [column.name, column.kind, column.nullable]),
    [["id", "i32", false], ["display name", "text", false], ["note", "text", true], ["version", "i32", false], [
      "mood",
      'enum "napi_entities"."mood"',
      false,
    ]],
  );
  assert.deepStrictEqual(described.relations, [{
    name: "Note",
    type: "hasMany",
    from: { schema: "napi_entities", table: "accounts" },
    to: { schema: "napi_entities", table: "notes" },
    columns: [["id", "account_id"]],
    period: null,
    conditionType: "all",
    onCondition: false,
    enforcement: null,
    deferrability: null,
  }]);
  // The key each Rust field serializes under, the field's own name where serde renames none.
  assert.deepStrictEqual(described.columns.map((column) => column.jsonKey), ["id", "name", "note", "version", "mood"]);
  assert.match(described.rust.entity, /account::Entity$/);
  assert.deepStrictEqual(Membership.describe().primaryKey, ["account_id", "team"]);
  assert.deepStrictEqual(graph("app.MixedNotes").describe().sources.map((source: { slot: string }) => source.slot), [
    "root",
    "Req",
    "Opt",
  ]);
});

// [spec:pgorm:req:bindings.entity-reads+3/test]
test("a query reads records through Select's terminals, keyed by SQL column", async () => {
  await seed();
  const busy = await Account.find().where(Account.col("mood").eq("busy")).all(pool());
  assert.deepStrictEqual(busy, [{ id: 2, "display name": "Bob", note: "b", version: 1, mood: "busy" }]);
  assert.ok(Object.isFrozen(busy[0]));
  const ordered = await Account.find().orderBy(Account.col("id").desc()).limit(2).all(pool());
  assert.deepStrictEqual(ordered.map((record) => record.id), [3, 2]);
  assert.equal((await Account.find().where(Account.col("id").gte(2)).orderBy(Account.col("id").asc()).one(pool())).id, 2);
  assert.equal((await Account.find().orderBy(Account.col("id").asc()).offset(1).optional(pool()))?.id, 2);
  assert.equal(await Account.find().where(Account.col("id").gt(9)).optional(pool()), null);
  await assert.rejects(Account.find().where(Account.col("id").gt(9)).one(pool()), DecodeError);
  assert.match(Account.find().inspect("one").sql, /LIMIT \$1$/);
  assert.match(Account.col("mood").eq("busy").inspect().sql, /= CAST\(\$1::text AS napi_entities\.mood\)/);
  assert.throws(() => Account.col("mood").eq(null as never), refused(/compared with null is never true/));
  assert.throws(() => Account.col("id").eq("one"), ConstructionError);
  assert.throws(() => Account.col("nickname"), refused(/no column "nickname"/));
  const quantified = await Account.find().where(Account.col("id").eq(any([1, 3]))).orderBy(Account.col("id").asc()).all(pool());
  assert.deepStrictEqual(quantified.map((record) => record.id), [1, 3]);
  assert.deepStrictEqual((await Account.find().where(Account.col("id").ne(all([1, 3]))).all(pool())).map((record) => record.id), [2]);
  const counted = await pool().one("SELECT count(*) AS n FROM napi_entities.accounts WHERE id = $1", [2]);
  assert.equal(counted.n, 1n);
});

// [spec:pgorm:req:bindings.entity-reads+3/test]
test("one and optional narrow a row limit and never widen one of zero", async () => {
  await seed();
  const none = Account.find().orderBy(Account.col("id").asc()).limit(0);
  assert.deepStrictEqual(await none.all(pool()), []);
  assert.equal(await none.optional(pool()), null);
  await assert.rejects(none.one(pool()), DecodeError);
  for (const terminal of ["one", "optional"] as const) {
    const { sql, values } = none.inspect(terminal);
    assert.match(sql, /LIMIT \$1$/);
    assert.equal(String(values.at(-1)?.value), "0");
  }
  const five = Account.find().orderBy(Account.col("id").asc()).limit(5);
  assert.equal((await five.one(pool())).id, 1);
  assert.equal(String(five.inspect("one").values.at(-1)?.value), "1");
});

// [spec:pgorm:req:bindings.entity-writes/test]
test("an ActiveModel holds each column's state, and its writes run the application's hooks", async () => {
  const fresh = Account.active();
  assert.deepStrictEqual(fresh.get("version"), { state: "set", value: 1 });
  assert.deepStrictEqual(fresh.get("mood"), { state: "set", value: "calm" });
  assert.deepStrictEqual(fresh.get("note"), { state: "notSet" });
  const ann = fresh.set("id", 1).set("display name", "Ann").set("note", "x").notSet("note");
  assert.deepStrictEqual(ann.get("note"), { state: "notSet" });
  assert.deepStrictEqual(fresh.get("display name"), { state: "notSet" });
  const inserted = await ann.insert(pool());
  assert.deepStrictEqual(inserted, { id: 1, "display name": "Ann|before", note: null, version: 1, mood: "calm" });
  const active = Account.intoActive(inserted);
  assert.deepStrictEqual(active.get("display name"), { state: "unchanged", value: "Ann|before" });
  const updated = await active.set("note", "seen").update(pool());
  assert.equal(updated.version, 2);
  assert.equal(updated.note, "seen");
  assert.deepStrictEqual(active.reset("note").get("note"), { state: "set", value: null });
  assert.deepStrictEqual(active.set("note", "y").get("note"), { state: "set", value: "y" });
  await assert.rejects(
    Account.active().set("id", 2).set("display name", "reject_before").insert(pool()),
    refused(/before_save refused/),
  );
  await assert.rejects(
    Account.active().set("id", 3).set("display name", "reject_after").insert(pool()),
    refused(/after_save refused/),
  );
  assert.deepStrictEqual((await Account.find().orderBy(Account.col("id").asc()).all(pool())).map((record) => record.id), [1, 3]);
  const doomed = await Account.active().set("id", 99).set("display name", "Zed").insert(pool());
  await assert.rejects(Account.intoActive(doomed).delete(pool()), refused(/before_delete refused/));
  assert.equal(await Account.intoActive(inserted).delete(pool()), 1);
  assert.throws(() => Account.active().set("id", "x"), ConstructionError);
  assert.throws(() => Account.active().set("mood", "wild"), ConstructionError);
});

// [spec:pgorm:req:bindings.entities+5/test]    a registered entity whose columns are domains, over an
// integer, over an enum and an array of one over text, describes, reads, writes and compares
// through them as through their base types
// [spec:pgorm:req:bindings.schema-domains+2/test]    a domain column reports, binds and reads as the
// kind it is over, and a check refuses a value written to it as a CheckViolationError naming the
// domain
// [spec:pgorm:req:bindings.entity-reads+3/test]    a column whose domain is over an enum compares as
// the enum
test("a registered entity's domain columns read, write and compare as the types they are over", async () => {
  assert.deepStrictEqual(
    Parcel.describe().columns.map((column: { name: string; kind: string | null }) => [column.name, column.kind]),
    [["id", "i32"], ["qty", "i32"], ["mood", 'enum "napi_entities"."mood"'], ["codes", "text[]"]],
  );
  const rows: [number, number, string, string[]][] = [
    [1, 5, "calm", ["A", "B"]],
    [2, 9, "busy", []],
    [3, 2, "calm", ["C"]],
  ];
  for (const [id, qty, mood, codes] of rows) {
    await Parcel.active().set("id", id).set("qty", qty).set("mood", mood).set("codes", codes).insert(pool());
  }
  const byId = Parcel.col("id").asc();
  const all = await Parcel.find().orderBy(byId).all(pool());
  assert.deepStrictEqual(all, rows.map(([id, qty, mood, codes]) => ({ id, qty, mood, codes })));
  const ids = (records: readonly EntityRecord[]) => records.map((record) => record.id);
  assert.deepStrictEqual(ids(await Parcel.find().where(Parcel.col("qty").gt(4)).orderBy(byId).all(pool())), [1, 2]);
  assert.deepStrictEqual(ids(await Parcel.find().where(Parcel.col("mood").eq("busy")).all(pool())), [2]);
  assert.deepStrictEqual(ids(await Parcel.find().where(Parcel.col("mood").lt("busy")).orderBy(byId).all(pool())), [1, 3]);
  assert.deepStrictEqual(ids(await Parcel.find().where(Parcel.col("mood").ne("calm")).all(pool())), [2]);
  assert.match(
    Parcel.col("mood").eq("busy").inspect().sql,
    /^SELECT CAST\("parcels"\."mood" AS napi_entities\.mood\) = CAST\(\$1::text AS napi_entities\.mood\)$/,
  );

  const updated = await Parcel.intoActive(all[0]).set("qty", 6).set("codes", ["Z"]).update(pool());
  assert.deepStrictEqual(updated, { id: 1, qty: 6, mood: "calm", codes: ["Z"] });
  const defaulted = await Parcel.active().set("id", 4).set("mood", "busy").set("codes", []).insert(pool());
  assert.equal(defaulted.qty, 1);

  const refusals: [number, string, string[], string, string][] = [
    [0, "calm", [], "quantity", "positive"],
    [1, "calm", ["A", ""], "code", "code_check"],
  ];
  for (const [qty, mood, codes, domain, constraint] of refusals) {
    await assert.rejects(
      Parcel.active().set("id", 5).set("qty", qty).set("mood", mood).set("codes", codes).insert(pool()),
      (error: unknown) => {
        assert.ok(error instanceof CheckViolationError, `expected a CheckViolationError, got ${error}`);
        assert.equal(error.datatype, domain);
        assert.equal(error.constraint, constraint);
        return true;
      },
    );
  }
  assert.deepStrictEqual(ids(await Parcel.find().orderBy(byId).all(pool())), [1, 2, 3, 4]);
});

// [spec:pgorm:req:bindings.entity-reads+3/test]    a column whose domain is over an enum is a
// member of a list of values and of a subquery's rows as the enum, either way round, where the
// column's plain expression is refused with 42883
// [spec:pgorm:req:bindings.pipeline-sources+3/test]    a registered column a pipeline reads
// compares as the enum, under its table and under a source's name, where the column named by text
// is refused with 42883
test("a registered column whose domain is over an enum is a member, and a pipeline's column, as the enum", async () => {
  for (const [id, mood] of [[1, "calm"], [2, "busy"], [3, "calm"]] as const) {
    await Parcel.active().set("id", id).set("mood", mood).set("codes", []).insert(pool());
  }
  const undefinedOperator = (error: unknown) => {
    assert.ok(error instanceof DatabaseError, `expected a DatabaseError, got ${error}`);
    assert.equal(error.sqlstate, "42883");
    return true;
  };
  const ids = (records: readonly EntityRecord[]) => records.map((record) => record.id).sort();
  const byId = Parcel.col("id").asc();
  const mood = Parcel.col("mood");
  const busy = select(mood).from(new Table("parcels", { schema: "napi_entities" })).where(Parcel.col("id").eq(2));
  assert.match(
    mood.isIn(busy).inspect().sql,
    /^SELECT CAST\("parcels"\."mood" AS napi_entities\.mood\) IN \(SELECT CAST\("parcels"\."mood" AS napi_entities\.mood\) FROM/,
  );
  assert.deepStrictEqual(ids(await Parcel.find().where(mood.isIn(busy)).all(pool())), [2]);
  assert.deepStrictEqual(ids(await Parcel.find().where(mood.isNotIn(busy)).orderBy(byId).all(pool())), [1, 3]);
  assert.deepStrictEqual(ids(await Parcel.find().where(mood.isIn(["busy"])).all(pool())), [2]);
  assert.deepStrictEqual(ids(await Parcel.find().where(mood.isNotIn(["busy"])).all(pool())), [1, 3]);
  await assert.rejects(Parcel.find().where(mood.isIn([bind("busy")])).all(pool()), undefinedOperator);
  const bare = Parcel.find().where(statementCol("mood", { table: "parcels", schema: "napi_entities" }).isIn(busy));
  await assert.rejects(bare.all(pool()), undefinedOperator);

  const pipelineId = pl.col("parcels", "id");
  const rows = async (pipeline: pl.Pipeline) =>
    (await pool().query(pipeline.select(pipelineId).sort(pipelineId))).map((row) => row.id);
  const typed = pl.col("parcels", mood);
  assert.match(
    pl.from(Parcel).filter(typed.eq("busy")).inspect().sql,
    /CAST\(mood AS napi_entities\.mood\) = CAST\(\$1::text AS napi_entities\.mood\)/,
  );
  assert.deepStrictEqual(await rows(pl.from(Parcel).filter(typed.eq("busy"))), [2]);
  assert.deepStrictEqual(await rows(pl.from(Parcel).filter(typed.lt("busy"))), [1, 3]);
  const peer = pl.source(Parcel).named("peer");
  const joined = pl.from(Parcel).join(peer, typed.eq(pl.col("peer", mood))).filter(pl.col("peer", "id").eq(3));
  assert.deepStrictEqual(await rows(joined), [1, 3]);
  await assert.rejects(rows(pl.from(Parcel).filter(pl.col("parcels", "mood").eq("busy"))), undefinedOperator);
});

// [spec:pgorm:req:bindings.entities+5/test]    a money column, and an array of it, is a decimal column:
// described as one, set, compared and read back as a Decimal through the entity's own casts, and
// compared as money by a pipeline that reads the registered column, where the column named by text
// takes no Decimal
test("a registered entity's money columns bind and read as decimals", async () => {
  assert.deepStrictEqual(
    Till.describe().columns.map((column: { name: string; kind: string | null }) => [column.name, column.kind]),
    [["id", "i32"], ["takings", "decimal"], ["floats", "decimal[]"]],
  );
  const written = await Till.active().set("id", 1).set("takings", new Decimal("12.345"))
    .set("floats", [new Decimal("1.5"), new Decimal("2")]).insert(pool());
  assert.deepStrictEqual([String(written.takings), (written.floats as Decimal[]).map(String)], ["12.35", ["1.50", "2.00"]]);
  assert.equal(Till.tagged(written, "takings").kind, "decimal");
  await Till.active().set("id", 2).set("takings", new Decimal("3")).insert(pool());

  const takings = Till.col("takings");
  const ids = (records: readonly EntityRecord[]) => records.map((record) => record.id).sort();
  assert.deepStrictEqual(ids(await Till.find().where(takings.eq(new Decimal("12.35"))).all(pool())), [1]);
  assert.deepStrictEqual(
    ids(await Till.find().where(takings.isIn([new Decimal("3"), new Decimal("12.35")])).all(pool())),
    [1, 2],
  );
  assert.deepStrictEqual(
    ids(await Till.find().where(Till.col("floats").eq([new Decimal("1.5"), new Decimal("2")])).all(pool())),
    [1],
  );

  const column = pl.col("tills", takings);
  const tillId = pl.col("tills", "id");
  const piped = async (pipeline: pl.Pipeline) =>
    (await pool().query(pipeline.select(tillId).sort(tillId))).map((row) => row.id);
  assert.match(pl.from(Till).filter(column.gt(new Decimal("5"))).inspect().sql, /CAST\(\$1::numeric AS money\)/);
  assert.deepStrictEqual(await piped(pl.from(Till).filter(column.gt(new Decimal("5")))), [1]);
  assert.deepStrictEqual(await piped(pl.from(Till).filterWith((binder) => column.lte(binder.bind(3)))), [2]);
  await assert.rejects(
    piped(pl.from(Till).filter(pl.col("tills", "takings").gt(new Decimal("5")))),
    refused(/cannot bind a `Decimal` value to Postgres type `money`/),
  );
});

// [spec:pgorm:req:bindings.column-operators+3/test]    a registered column's between, symmetric or
// not, and its distinctness tests run through its ColumnTrait: a money column's bounds cast to money,
// an enum's labels to the enum, and a column whose domain is over an enum tested as the enum, where
// the column's plain expression is refused; an expression bound is taken as written, null never
test("a registered column's between and distinctness tests compare under its casts", async () => {
  await seed();
  for (const [id, takings] of [[1, "12.35"], [2, "3"], [3, "7.5"]] as const) {
    await Till.active().set("id", id).set("takings", new Decimal(takings)).insert(pool());
  }
  for (const [id, mood] of [[1, "calm"], [2, "busy"]] as const) {
    await Parcel.active().set("id", id).set("mood", mood).set("codes", []).insert(pool());
  }
  const ids = async (query: { all(pool: Pool): Promise<readonly EntityRecord[]> }) =>
    (await query.all(pool())).map((record) => record.id).sort();
  const takings = Till.col("takings");
  const [low, high] = [new Decimal("3"), new Decimal("8")];
  assert.match(
    takings.between(low, high).inspect().sql,
    /BETWEEN CAST\(\$1::numeric AS money\) AND CAST\(\$2::numeric AS money\)/,
  );
  assert.deepStrictEqual(await ids(Till.find().where(takings.between(low, high))), [2, 3]);
  assert.deepStrictEqual(await ids(Till.find().where(takings.notBetween(low, high))), [1]);
  assert.deepStrictEqual(await ids(Till.find().where(takings.between(high, low))), []);
  assert.deepStrictEqual(await ids(Till.find().where(takings.between(high, low, { symmetric: true }))), [2, 3]);
  assert.deepStrictEqual(await ids(Till.find().where(takings.notBetween(high, low, { symmetric: true }))), [1]);
  assert.deepStrictEqual(await ids(Till.find().where(takings.isDistinctFrom(low))), [1, 3]);
  assert.deepStrictEqual(await ids(Till.find().where(takings.isNotDistinctFrom(low))), [2]);
  assert.deepStrictEqual(await ids(Till.find().where(takings.between(takings, takings))), [1, 2, 3]);

  const mood = Account.col("mood");
  assert.deepStrictEqual(await ids(Account.find().where(mood.between("busy", "busy"))), [2]);
  assert.deepStrictEqual(await ids(Account.find().where(mood.notBetween("busy", "calm", { symmetric: true }))), []);
  assert.deepStrictEqual(await ids(Account.find().where(mood.isDistinctFrom("busy"))), [1, 3]);
  assert.deepStrictEqual(await ids(Account.find().where(mood.isNotDistinctFrom("busy"))), [2]);

  const temper = Parcel.col("mood");
  assert.deepStrictEqual(await ids(Parcel.find().where(temper.between("calm", "calm"))), [1]);
  assert.deepStrictEqual(await ids(Parcel.find().where(temper.between("busy", "calm", { symmetric: true }))), [1, 2]);
  assert.deepStrictEqual(await ids(Parcel.find().where(temper.isDistinctFrom("calm"))), [2]);
  assert.deepStrictEqual(await ids(Parcel.find().where(temper.isNotDistinctFrom("busy"))), [2]);
  const plain = statementCol("mood", { table: "parcels", schema: "napi_entities" });
  for (const bare of [plain.between("calm", "busy"), plain.isDistinctFrom("calm")]) {
    await assert.rejects(Parcel.find().where(bare).all(pool()), (error: unknown) => {
      assert.ok(error instanceof DatabaseError, `expected a DatabaseError, got ${error}`);
      assert.equal(error.sqlstate, "42883");
      return true;
    });
  }
  assert.throws(() => takings.between(null as never, high), refused(/compared with null is never true/));
  assert.throws(() => takings.isDistinctFrom(null as never), refused(/compared with null is never true/));
});

// [spec:pgorm:req:bindings.entity-reads+3/test]    any(..) / all(..) over a list compares through the
// column's ColumnTrait, the list one array of its declared kind cast as eq_any's is: money as money[],
// a column whose domain is over an enum as the enum, where the plain expression is refused; over an
// expression it is compared as written
// [spec:pgorm:req:bindings.column-operators+3/test]    a registered array column's containment and
// overlap run through its ColumnTrait
// [spec:pgorm:req:bindings.quantifiers+2/test]    an empty list is typed by the registered column, = ANY
// of it false and <> ALL true, and refused as it is used where no registered column types it
test("a registered column's quantified comparisons and containment compare under its casts", async () => {
  await seed();
  for (const [id, takings] of [[1, "12.35"], [2, "3"], [3, "7.5"]] as const) {
    await Till.active().set("id", id).set("takings", new Decimal(takings)).insert(pool());
  }
  for (const [id, mood, codes] of [[1, "calm", ["a", "b"]], [2, "busy", ["c"]]] as const) {
    await Parcel.active().set("id", id).set("mood", mood).set("codes", [...codes]).insert(pool());
  }
  const ids = async (query: { all(pool: Pool): Promise<readonly EntityRecord[]> }) =>
    (await query.all(pool())).map((record) => record.id).sort();
  const takings = Till.col("takings");
  assert.match(takings.eq(any([new Decimal("3")])).inspect().sql, /= ANY\(CAST\(\$1::numeric\[\] AS money\[\]\)\)/);
  assert.deepStrictEqual(await ids(Till.find().where(takings.eq(any([new Decimal("3"), new Decimal("12.35")])))), [1, 2]);
  assert.deepStrictEqual(await ids(Till.find().where(takings.gt(all([new Decimal("1"), new Decimal("5")])))), [1, 3]);
  assert.deepStrictEqual(await ids(Till.find().where(takings.eq(any([])))), []);
  assert.deepStrictEqual(await ids(Till.find().where(takings.ne(all([])))), [1, 2, 3]);
  const empty = any([]);
  assert.throws(() => statementCol("x").eq(empty), ConstructionError);
  assert.deepStrictEqual(await ids(Account.find().where(Account.col("mood").ne(all(["busy"])))), [1, 3]);
  const temper = Parcel.col("mood");
  assert.deepStrictEqual(await ids(Parcel.find().where(temper.eq(any(["busy"])))), [2]);
  assert.deepStrictEqual(await ids(Parcel.find().where(temper.gt(all(["calm"])))), [2]);
  const plain = statementCol("mood", { table: "parcels", schema: "napi_entities" });
  await assert.rejects(Parcel.find().where(plain.eq(any(["busy"]))).all(pool()), (error: unknown) => {
    assert.ok(error instanceof DatabaseError, `expected a DatabaseError, got ${error}`);
    assert.equal(error.sqlstate, "42883");
    return true;
  });
  const identifier = Account.col("id");
  const versions = select(Account.col("version")).from(new Table("accounts", { schema: "napi_entities" }));
  assert.deepStrictEqual(await ids(Account.find().where(identifier.eq(any(versions)))), [1]);

  const codes = Parcel.col("codes");
  assert.deepStrictEqual(await ids(Parcel.find().where(codes.contains(["a"]))), [1]);
  assert.deepStrictEqual(await ids(Parcel.find().where(codes.containedBy(["a", "b", "c"]))), [1, 2]);
  assert.deepStrictEqual(await ids(Parcel.find().where(codes.overlaps(["c", "d"]))), [2]);
});

// [spec:pgorm:req:bindings.errors+5/test]    a column the entity cannot read as its field's type is a
// DecodeError naming what the driver was reading and the decoder's reason, as Python reports it
test("a decode refusal names the column the driver read and the decoder's reason", async () => {
  await pool().execute("ALTER TABLE napi_entities.notes ALTER COLUMN account_id TYPE text USING account_id::text");
  try {
    await pool().execute("INSERT INTO napi_entities.notes VALUES (1, 'one', 'b')");
    await assert.rejects(Note.find().all(pool()), (error: unknown) => {
      assert.ok(error instanceof DecodeError, `expected a DecodeError, got ${error}`);
      assert.equal(
        error.message,
        "error deserializing column 1: cannot convert between the Rust type `i32` and the Postgres type `text`",
      );
      return true;
    });
  } finally {
    await pool().execute("TRUNCATE napi_entities.notes");
    await pool().execute("ALTER TABLE napi_entities.notes ALTER COLUMN account_id TYPE int4 USING account_id::int4");
  }
});

// [spec:pgorm:req:bindings.entity-reads+3/test]    any(..) / all(..) over a list of ranges compared
// with a column of a range type the schema created is the array of their texts cast to the type's
// array
// [spec:pgorm:req:bindings.quantifiers+2/test]    a list of ranges, of no kind the list alone names,
// is typed by the registered column it meets
test("a registered created range's quantified comparisons bind as the array of the ranges' texts", async () => {
  for (const [id, lower, upper] of [[1, 1.5, 2.5], [2, 5, 6]] as const) {
    await Booking.active().set("id", id).set("span", new Range(lower, upper)).insert(pool());
  }
  const ids = async (query: { all(pool: Pool): Promise<readonly EntityRecord[]> }) =>
    (await query.all(pool())).map((record) => record.id).sort();
  const span = Booking.col("span");
  assert.match(
    span.eq(any([new Range(1.5, 2.5)])).inspect().sql,
    /= ANY\(CAST\(\$1::text\[\] AS napi_entities\.floatrange\[\]\)\)/,
  );
  assert.deepStrictEqual(await ids(Booking.find().where(span.eq(any([new Range(1.5, 2.5), new Range(7, 8)])))), [1]);
  assert.deepStrictEqual(await ids(Booking.find().where(span.overlaps(any([new Range(2, 3), new Range(5.5, 5.6)])))), [1, 2]);
  assert.deepStrictEqual(await ids(Booking.find().where(span.overlaps(all([new Range(2, 3), new Range(1, 1.6)])))), [1]);
});

// [spec:pgorm:req:bindings.created-ranges+3/test]    a registered column of an array of a created
// range takes an array of the kind's values, the empty array and null elements included, as the array
// of their texts cast to the type's array, compares through the column's casts, and reads back as an
// array, a declared Value of the array's kind included
test("a registered created range array binds as the array of the ranges' texts", async () => {
  await Booking.active().set("id", 1).set("span", new Range(1, 2)).set("spares", [new Range(1.5, 2.5), new Range(3, 4)])
    .insert(pool());
  await Booking.active().set("id", 2).set("span", new Range(1, 2)).set("spares", []).insert(pool());
  const found = await Booking.find().where(Booking.col("id").eq(1)).one(pool());
  assert.deepStrictEqual(found.spares, [new Range(1.5, 2.5), new Range(3, 4)]);
  const ids = async (query: { all(pool: Pool): Promise<readonly EntityRecord[]> }) =>
    (await query.all(pool())).map((record) => record.id).sort();
  const spares = Booking.col("spares");
  assert.match(
    spares.eq([new Range(1.5, 2.5)]).inspect().sql,
    /= CAST\(\$1::text\[\] AS napi_entities\.floatrange\[\]\)/,
  );
  assert.deepStrictEqual(await ids(Booking.find().where(spares.eq([new Range(1.5, 2.5), new Range(3, 4)]))), [1]);
  assert.deepStrictEqual(await ids(Booking.find().where(spares.eq([]))), [2]);
  assert.deepStrictEqual(await ids(Booking.find().where(spares.ne([new Range(1.5, 2.5), null]))), [1, 2]);
  assert.deepStrictEqual(await ids(Booking.find().where(spares.contains([new Range(3, 4)]))), [1]);
  const moved = await Booking.intoActive(found).set("spares", Booking.tagged(found, "spares")).set("span", new Range(0, 1))
    .update(pool());
  assert.deepStrictEqual(moved.spares, found.spares);
  assert.throws(() => Booking.active().set("spares", ["nope"]), ConstructionError);
  assert.throws(() => Booking.active().set("span", Booking.tagged(found, "spares")), ConstructionError);
  const declared = Value.array(new CreatedRange("floatrange", "f64", { schema: "napi_entities" }), [new Range(5, 6)]);
  assert.deepStrictEqual(await ids(Booking.find().where(spares.ne(declared))), [1, 2]);
  assert.throws(() => Booking.active().set("spares", Value.array(new CreatedRange("floatrange", "f64"), [])), ConstructionError);
});

// [spec:pgorm:req:bindings.entity-writes/test]
test("a record's model converts and copies as Rust's does, and belongs to its entity", async () => {
  await seed();
  const bob = await Account.find().where(Account.col("id").eq(2)).one(pool());
  const renamed = Account.withValue(bob, "display name", "Robert");
  assert.equal(renamed["display name"], "Robert");
  assert.equal(bob["display name"], "Bob");
  const mood = Account.tagged(bob, "mood");
  assert.equal(mood.kind, "enum");
  assert.ok(mood.typeName instanceof TypeName);
  assert.equal(String(mood.typeName), '"napi_entities"."mood"');
  assert.deepStrictEqual(Account.intoActive(renamed).get("display name"), { state: "unchanged", value: "Robert" });
  assert.throws(() => Account.intoActive({ ...bob }), TypeError);
  const note = await Note.find().optional(pool());
  assert.ok(note);
  assert.throws(() => Account.intoActive(note), refused(/app.Note's, not app.Account's/));
  assert.throws(() => Account.update(Note.active()), refused(/app.Note's, not app.Account's/));
});

// [spec:pgorm:req:bindings.entity-versions+2/test]
// [spec:pgorm:req:bindings.entity-versions+2/test]
test("the version terminals return each written row before and after, no hook running", async () => {
  await seed();
  const ann = await Account.find().where(Account.col("id").eq(1)).one(pool());
  const change = await Account.update(Account.intoActive(ann).set("note", "changed")).returningChange(pool());
  assert.equal(change.old.note, null);
  assert.equal(change.new.note, "changed");
  assert.equal(change.new.version, 1);
  const changes = await Account.updateMany().set("note", "bulk").set("version", Account.col("version").add(10))
    .where(Account.col("mood").eq("calm")).returningChanges(pool());
  changes.sort((left, right) => Number(left.old.id) - Number(right.old.id));
  assert.deepStrictEqual(changes.map(({ old, new: now }) => [old.id, old.note, now.note, now.version]), [
    [1, "changed", "bulk", 11],
    [3, null, "bulk", 11],
  ]);
  await assert.rejects(Account.updateMany().set("note", "x").returningChanges(pool()), refused(/where\(\.\.\) or allRows\(\)/));
  await assert.rejects(Account.updateMany().allRows().returningChanges(pool()), ConstructionError);
  assert.equal((await Account.updateMany().set("note", "all").allRows().returningChanges(pool())).length, 3);
  const renamed = Conflict.on("id").update("display name");
  const updated = await Account.insert(Account.active().set("id", 1).set("display name", "Anne")).onConflict(renamed)
    .returningUpsert(pool());
  assert.ok(updated?.kind === "updated");
  assert.equal(updated.old["display name"], "Ann");
  assert.equal(updated.new["display name"], "Anne");
  const inserted = await Account.insert(Account.active().set("id", 7).set("display name", "Gil")).onConflict(renamed)
    .returningUpsert(pool());
  assert.ok(inserted?.kind === "inserted");
  assert.equal(inserted.new["display name"], "Gil");
  assert.equal(
    await Account.insert(Account.active().set("id", 7).set("display name", "Hal")).onConflict(Conflict.doNothing())
      .returningUpsert(pool()),
    null,
  );
  const batch = await Account.insertMany([
    Account.active().set("id", 2).set("display name", "Bobby"),
    Account.active().set("id", 8).set("display name", "Ivy"),
  ]).onConflict(renamed).returningUpserts(pool());
  assert.deepStrictEqual(batch.map((row) => [row.kind, row.new["display name"]]), [["updated", "Bobby"], ["inserted", "Ivy"]]);
  assert.deepStrictEqual(await Account.insertMany([]).returningUpserts(pool()), []);
});

// [spec:pgorm:req:bindings.entity-graphs+3/test]
test("a registered graph decodes its root and slots, an absent optional slot null", async () => {
  await seed();
  type Pair = [Record<string, unknown>, Record<string, unknown> | null];
  const optional = await graph("app.AccountNotes").find({ aliases: ["n"] }).orderBy(Account.col("id").asc()).all(pool()) as Pair[];
  assert.deepStrictEqual(optional.map(([account, note]) => [account.id, note?.body ?? null]), [
    [1, "a1"],
    [1, "a2"],
    [2, "b1"],
    [3, null],
  ]);
  const required = await graph("app.RequiredNotes").find().all(pool());
  assert.equal(required.length, 3);
  const roots = await graph("app.AccountOnly").find().all(pool());
  assert.equal(roots.length, 3);
  assert.ok(!Array.isArray(roots[0]));
  const mixed = graph("app.MixedNotes").find();
  assert.deepStrictEqual(mixed.aliases, ["g1", "g2"]);
  const query = graph("app.AccountNotes").find({ aliases: ["n"] });
  const filtered = await query.where(query.col(1, "body").eq("b1")).optional(pool()) as Pair | null;
  assert.equal(filtered?.[0].id, 2);
  assert.equal(await query.where(query.col(0, "id").eq(99)).optional(pool()), null);
  assert.throws(() => query.col(2, "id"), refused(/no source 2/));
  assert.throws(() => query.col(1, "nope"), refused(/no column "nope"/));
  assert.throws(() => graph("app.AccountNotes").find({ aliases: ["a", "b"] }), refused(/joins 1 slots, and 2 aliases/));
  assert.throws(() => graph("app.AccountNotes").find({ aliases: ["accounts"] }), refused(/names a source twice/));
});

// [spec:pgorm:req:bindings.entity-graphs+3/test]
test("a registered graph's cursor pages by keyset, resuming inside a root's slot rows", async () => {
  await seed();
  const cursor = graph("app.AccountNotes").find({ aliases: ["n"] }).cursor("id");
  const first = await cursor.first(1).all(pool()) as [Record<string, unknown>, Record<string, unknown> | null][];
  assert.deepStrictEqual(first.map(([account, note]) => [account.id, note?.id]), [[1, 10]]);
  const resumed = await cursor.afterWith(1, 10).first(5).all(pool()) as [Record<string, unknown>, unknown][];
  assert.deepStrictEqual(resumed.map(([account]) => account.id), [1, 2, 3]);
  const skipped = await cursor.after(1).first(5).all(pool()) as [Record<string, unknown>, unknown][];
  assert.deepStrictEqual(skipped.map(([account]) => account.id), [2, 3]);
  const last = await cursor.last(2).all(pool()) as [Record<string, unknown>, unknown][];
  assert.deepStrictEqual(last.map(([account]) => account.id), [2, 3]);
  const descending = await cursor.desc().first(1).all(pool()) as [Record<string, unknown>, unknown][];
  assert.deepStrictEqual(descending.map(([account]) => account.id), [3]);
  assert.throws(() => cursor.afterWith(1), refused(/takes 2 values, and 1 were given/));
  assert.throws(() => cursor.after("x"), ConstructionError);
});

// [spec:pgorm:req:bindings.entities+5/test]
// [spec:pgorm:req:bindings.registered-pools/test]
test("an entity's operations run on a connection or a transaction, with a statement's refusals and aborts", async () => {
  await seed();
  await assert.rejects(
    pool().transaction(async (transaction) => {
      await Account.active().set("id", 50).set("display name", "Tx").insert(transaction);
      assert.equal((await Account.find().where(Account.col("id").eq(50)).all(transaction)).length, 1);
      throw new Error("roll back");
    }),
    /roll back/,
  );
  assert.equal(await Account.find().where(Account.col("id").eq(50)).optional(pool()), null);
  await pool().connection(async (connection) => {
    const slow = pool().query("SELECT 1");
    const reads = [Account.find().all(connection), Account.find().all(connection)];
    const outcomes = await Promise.allSettled(reads);
    assert.ok(outcomes.some((outcome) => outcome.status === "rejected" && outcome.reason instanceof LifecycleError));
    await slow;
  });
  const signal = AbortSignal.abort(new Error("stop"));
  await assert.rejects(Account.find().all(pool(), { signal }), /stop/);
  await assert.rejects(Account.find().all({} as never), TypeError);
});

// [spec:pgorm:req:bindings.pipeline-sources+3/test]
test("a pipeline reads a registered entity's table, and selects registered sources into their models", async () => {
  await seed();
  const ids = await pool().query(pl.from(Account).filter(pl.col("accounts", "id").gt(1)).sort(pl.col("accounts", "id")));
  assert.deepStrictEqual(ids.map((row) => row.id), [2, 3]);
  const joined = pl.from(Account)
    .join(pl.source(Note).named("n"), pl.col("accounts", "id").eq(pl.col("n", "account_id")), { kind: "left" })
    .sort(pl.col("accounts", "id"), pl.col("n", "id"));
  const rows = await joined.selectSources(pl.sources("app.AccountWithNote"), { qualifiers: ["accounts", "n"] }).all(pool());
  assert.deepStrictEqual(rows.map(([account, note]) => [account?.id, note?.body ?? null]), [
    [1, "a1"],
    [1, "a2"],
    [2, "b1"],
    [3, null],
  ]);
  const [account, note] = rows[0]!;
  assert.ok(account && note && Object.isFrozen(account));
  assert.deepStrictEqual(Account.intoActive(account).get("display name"), { state: "unchanged", value: "Ann" });
  assert.equal(Note.tagged(note, "body").kind, "text");
  const single = pl.from(Account).sort(pl.col("accounts", "id")).selectSources(pl.sources("app.SingleAccount"));
  assert.deepStrictEqual((await single.all(pool())).map(([only]) => only?.id), [1, 2, 3]);
  assert.equal((await single.optional(pool()))?.[0]?.id, 1);
  assert.equal((await single.one(pool()))[0]?.id, 1);
  const none = pl.from(Account).filter(pl.col("accounts", "id").gt(9)).selectSources(pl.sources("app.SingleAccount"));
  assert.equal(await none.optional(pool()), null);
  await assert.rejects(none.one(pool()), DecodeError);
  assert.deepStrictEqual(pl.sources("app.SixSources").describe().entities, [
    "app.Account",
    "app.Note",
    "app.Note",
    "app.Note",
    "app.Note",
    "app.Note",
  ]);
  assert.deepStrictEqual(pl.sourceTuples(), ["app.AccountWithNote", "app.SingleAccount", "app.SixSources"]);
});

// [spec:pgorm:req:bindings.pipeline-sources+3/test]
test("a selection refuses a reshaped pipeline, a wrong count of qualifiers and a tuple the module does not register", async () => {
  const reshaped = pl.from(Account).select(pl.col("accounts", "id")).selectSources(pl.sources("app.SingleAccount"));
  assert.throws(() => reshaped.inspect(), refused(/select/));
  await assert.rejects(reshaped.all(pool()), refused(/select/));
  assert.throws(
    () => pl.from(Account).selectSources(pl.sources("app.AccountWithNote"), { qualifiers: ["accounts"] }),
    refused(/2 sources, and 1 qualifiers/),
  );
  assert.throws(() => pl.sources("app.Missing"), refused(/no source tuple is registered as "app.Missing"/));
  assert.throws(() => pl.from(Account).selectSources({} as never), TypeError);
});
