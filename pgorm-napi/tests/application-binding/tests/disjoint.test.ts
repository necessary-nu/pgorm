// The application's `#[derive(Disjoint)]` enum, registered with its
// entities, through its own native module, against a live server in either
// runtime: its description, its schema, and its reads and writes through its
// entities' records and ActiveModels.

import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import {
  addDisjointKind,
  ConstructionError,
  createDisjoint,
  DecodeError,
  disjointEntities,
  disjointEntity,
  entity,
  Pool,
  removeDisjointKind,
  type UnkindedDisjointEntity,
} from "../../../lib/index.js";
import { scratchDatabase } from "../../support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

const Shipments = disjointEntity("app.Shipments");
const Shipment = entity("app.Shipment");
const Packet = entity("app.Packet");
const Pallet = entity("app.Pallet");

before(async () => {
  database = await scratchDatabase("pgorm_napi_shipments");
  shared = new Pool(database.dsn, { maxSize: 4 });
});

beforeEach(async () => {
  await pool().execute("DROP SCHEMA IF EXISTS napi_shipments CASCADE");
  await pool().execute("CREATE SCHEMA napi_shipments");
  for (const statement of createDisjoint(Shipments)) await pool().execute(statement);
});

after(async () => {
  await shared?.close();
  await database?.drop();
});

function base(label: string, values: Record<string, unknown> = {}) {
  let active = Shipment.active().set("label", label);
  for (const [column, value] of Object.entries(values)) active = active.set(column, value);
  return active;
}

async function seed() {
  const boxed = await Shipments.insert(base("box"), "packet", Packet.active().set("weight_g", 500)).returningModel(pool());
  const note = await Shipments.insert(base("note"), "letter").returningModel(pool());
  const crate = await Shipments.insert(
    base("crate"),
    "pallet",
    Pallet.active().set("volume_l", 40).set("fragile", null),
  ).returningModel(pool());
  return [boxed, note, crate];
}

// [spec:pgorm:req:bindings.disjoint+1/test]    a registered disjoint enum is listed and described under the declared keys beside its base and kind entities; a name the module does not register is refused; its schema is Schema::create_disjoint's
test("the registration describes the enum", () => {
  assert.deepStrictEqual(disjointEntities(), ["app.Listings", "app.Shipments"]);
  assert.deepStrictEqual(Shipments.describe(), {
    name: "app.Shipments",
    base: "app.Shipment",
    kindField: "kind",
    kindType: { schema: "napi_shipments", name: "shipment_kind" },
    kinds: [
      { value: "packet", model: "app.Packet" },
      { value: "letter", model: null },
      { value: "pallet", model: "app.Pallet" },
    ],
  });
  assert.throws(() => disjointEntity("app.Missing"), ConstructionError);
  const statements = createDisjoint(Shipments);
  assert.deepStrictEqual(statements.map((statement) => statement.constructor.name), [
    "CreateType",
    "CreateTable",
    "CreateTable",
    "CreateTable",
    "AlterTable",
  ]);
  assert.match(
    statements[0]!.inspect().sql,
    /^CREATE TYPE "napi_shipments"\."shipment_kind" AS ENUM \('packet', 'letter', 'pallet'\)$/,
  );
});

// [spec:pgorm:req:bindings.disjoint+1/test]    a registered disjoint enum's writes take its entities' ActiveModels and return each row as the base's record, the kind and the kind's record, which keep the Rust models they were read as; reads narrow, page, count, stream and read through a cursor on a pool and a connection
test("writes and reads through the registered entities", async () => {
  const [boxed, note, crate] = await seed();
  assert.deepStrictEqual(boxed, [{ id: 1, kind: "packet", label: "box" }, "packet", { shipment_id: 1, weight_g: 500 }]);
  assert.deepStrictEqual(note, [{ id: 2, kind: "letter", label: "note" }, "letter", null]);
  assert.deepStrictEqual(crate, [{ id: 3, kind: "pallet", label: "crate" }, "pallet", { id: 3, volume_l: 40, fragile: null }]);
  assert.ok(Object.isFrozen(boxed[0]));
  assert.deepStrictEqual(Shipment.intoActive(boxed[0]).get("label"), { state: "unchanged", value: "box" });
  assert.equal(Packet.tagged(boxed[2]!, "weight_g").value, 500);
  const everything = [boxed, note, crate];
  const ordered = Shipments.find().orderBy(Shipment.col("id").asc());
  assert.deepStrictEqual(await ordered.all(pool()), everything);
  await pool().connection(async (connection) => {
    assert.deepStrictEqual(await Shipments.findByKey({ id: 3 }).one(connection), crate);
    const streamed = [];
    for await (const row of ordered.stream(connection)) streamed.push(row);
    assert.deepStrictEqual(streamed, everything);
  });
  assert.deepStrictEqual(await ordered.kinds(["letter", "pallet"]).all(pool()), [note, crate]);
  assert.deepStrictEqual(await ordered.filterKind("packet", Packet.col("weight_g").gt(100)).all(pool()), [boxed]);
  assert.equal(await ordered.kind("packet").count(pool()), 1);
  const pages = [];
  for await (const page of ordered.paginate(2).pages(pool())) pages.push(page);
  assert.deepStrictEqual(pages, [[boxed, note], [crate]]);
  assert.deepStrictEqual(await Shipments.find().cursor("id").after(1).first(1).all(pool()), [note]);
});

// [spec:pgorm:req:bindings.disjoint+1/test]    a registered enum's update and change of kind write through its entities' ActiveModels, a miss being a DecodeError that changes nothing, and an ActiveModel of another registration, the kind column set, a kind row's key set and a row for a kind without a table are refused before anything is sent
test("updates, changes of kind and refusals", async () => {
  await seed();
  assert.deepStrictEqual(
    await Shipments.update(base("big box", { id: 1 }), "packet", Packet.active().set("weight_g", 900)).returningModel(pool()),
    [{ id: 1, kind: "packet", label: "big box" }, "packet", { shipment_id: 1, weight_g: 900 }],
  );
  await assert.rejects(Shipments.update(base("x", { id: 1 }), "letter").returningModel(pool()), DecodeError);
  await assert.rejects(Shipments.changeKind({ id: 2 }, "letter").returningModel(pool()), DecodeError);
  assert.deepStrictEqual(
    await Shipments.changeKind({ id: 1 }, "pallet", Pallet.active().set("volume_l", 12)).returningModel(pool()),
    [{ id: 1, kind: "pallet", label: "big box" }, "pallet", { id: 1, volume_l: 12, fragile: null }],
  );
  assert.equal(await Shipments.find().kind("packet").count(pool()), 0);
  for (const write of [
    () => Shipments.insert(Packet.active(), "letter"),
    () => Shipments.insert(base("x", { kind: "letter" }), "letter"),
    () => Shipments.insert(base("x"), "packet", Packet.active().set("shipment_id", 9)),
    () => Shipments.insert(base("x"), "letter", Packet.active()),
    () => Shipments.insert(base("x"), "packet", Pallet.active()),
    () => Shipments.update(base("x"), "letter"),
  ]) {
    assert.throws(write, ConstructionError);
  }
  assert.throws(() => Shipments.findByKey({ label: "x" }), TypeError);
  assert.throws(() => Shipments.insert({ label: "x" } as never, "letter"), TypeError);
});

/** Listings as an existing schema holds them: no kind column, each kind's table keyed by a foreign key onto the listing's. */
const LISTINGS = [
  "DROP SCHEMA IF EXISTS napi_listings CASCADE",
  "CREATE SCHEMA napi_listings",
  "CREATE TABLE napi_listings.listings (id integer PRIMARY KEY, title text NOT NULL)",
  "CREATE TABLE napi_listings.auctions (listing_id integer PRIMARY KEY REFERENCES napi_listings.listings (id), reserve integer NOT NULL)",
  "CREATE TABLE napi_listings.sales (id integer PRIMARY KEY REFERENCES napi_listings.listings (id), price integer NOT NULL)",
  "INSERT INTO napi_listings.listings VALUES (1, 'lamp'), (2, 'chair'), (3, 'desk')",
  "INSERT INTO napi_listings.auctions VALUES (1, 50)",
  "INSERT INTO napi_listings.sales VALUES (2, 80)",
];

const Listings: UnkindedDisjointEntity = disjointEntity("app.Listings");
const Listing = entity("app.Listing");
const Auction = entity("app.Auction");

// [spec:pgorm:req:bindings.disjoint-unkinded/test]    a downstream crate's unkinded enum registers through the same registration, describes itself with no kind field or kind type, reads each row as the kind whose table has its row, the unit kind where none does, its kinds named as its variants in snake_case, narrows by presence, and reports a row in two kinds' tables as a DecodeError
test("an unkinded enum registers and reads by presence", async () => {
  for (const statement of LISTINGS) await pool().execute(statement);
  assert.deepStrictEqual(Listings.describe(), {
    name: "app.Listings",
    base: "app.Listing",
    kindField: null,
    kindType: { schema: null, name: null },
    kinds: [
      { value: "auction", model: "app.Auction" },
      { value: "sale", model: "app.Sale" },
      { value: "draft", model: null },
    ],
  });
  const ordered = Listings.find().orderBy(Listing.col("id").asc());
  const everything = [
    [{ id: 1, title: "lamp" }, "auction", { listing_id: 1, reserve: 50 }],
    [{ id: 2, title: "chair" }, "sale", { id: 2, price: 80 }],
    [{ id: 3, title: "desk" }, "draft", null],
  ];
  assert.deepStrictEqual(await ordered.all(pool()), everything);
  assert.deepStrictEqual(await ordered.kind("draft").all(pool()), everything.slice(2));
  assert.deepStrictEqual(await ordered.filterKind("auction", Auction.col("reserve").gt(10)).all(pool()), everything.slice(0, 1));
  assert.equal(await Listings.find().kinds(["auction", "sale"]).count(pool()), 2);
  await pool().execute("INSERT INTO napi_listings.sales VALUES (1, 70)");
  await assert.rejects(Listings.findByKey({ id: 1 }).one(pool()), (error: unknown) => {
    assert.ok(error instanceof DecodeError, `expected a DecodeError, got ${error}`);
    assert.match(error.message, /of an unkinded disjoint entity has a row in/);
    return true;
  });
});

// [spec:pgorm:req:bindings.disjoint-evolve/test]    a registered enum's kind changes are the shape's, Schema's for the enum: a value the database holds and the enum does not is removed live, the kind type recreated with the enum's values in order
test("a registration's kind changes are the enum's", async () => {
  const added = addDisjointKind(Shipments, "pallet");
  assert.deepStrictEqual(added.steps[0]?.statements.map((statement) => statement.inspect().sql), [
    `ALTER TYPE "napi_shipments"."shipment_kind" ADD VALUE 'pallet' AFTER 'letter'`,
  ]);
  assert.equal(added.steps.length, 2);
  await pool().execute("ALTER TYPE napi_shipments.shipment_kind ADD VALUE 'ghost'");
  for (const step of removeDisjointKind(Shipments, "ghost").steps) {
    await pool().transaction(async (tx) => {
      for (const statement of step.statements) await tx.execute(statement);
    });
  }
  pool().clearStatementCaches();
  const [row] = await pool().query("SELECT enum_range(NULL::napi_shipments.shipment_kind)::text AS kinds");
  assert.deepStrictEqual(row, { kinds: "{packet,letter,pallet}" });
  assert.equal((await seed()).length, 3);
});

// [spec:pgorm:req:bindings.disjoint-unkinded/test]    a registered unkinded enum's writes and schema are refused with a ConstructionError naming its base read-only, before anything is sent, and its type offers none of them
test("an unkinded registration is read-only", async () => {
  for (const statement of LISTINGS) await pool().execute(statement);
  const writable = disjointEntity("app.Listings");
  const draft = Listing.active().set("id", 9).set("title", "rug");
  for (const write of [
    () => writable.insert(draft, "draft"),
    () => writable.insert(draft, "auction", Auction.active().set("reserve", 1)),
    () => writable.update(Listing.active().set("id", 1).set("title", "x"), "draft"),
    () => writable.changeKind({ id: 1 }, "draft"),
    () => createDisjoint(writable),
    () => addDisjointKind(writable, "draft"),
  ]) {
    assert.throws(write, (error: unknown) => {
      assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
      assert.match(error.message, /is unkinded: .* so it is read-only/);
      return true;
    });
  }
  // @ts-expect-error an unkinded registration has no writes
  assert.equal(typeof Listings.insert, "function");
  // @ts-expect-error nor a schema
  assert.throws(() => createDisjoint(Listings), ConstructionError);
  assert.equal((await pool().one("SELECT count(*)::int AS n FROM napi_listings.listings")).n, 3);
});
