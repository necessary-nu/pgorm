// Disjoint models declared over runtime models, in either runtime: the
// declaration and its refusals, the statements pgorm's shape builds, and the
// schema, reads and writes on a live server.

import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import {
  column,
  ConstructionError,
  createDisjoint,
  DecodeError,
  DDL,
  disjoint,
  ForeignKeyViolationError,
  model,
  type Model,
  NotNullViolationError,
  Pool,
  TypeName,
} from "../lib/index.js";
import { scratchDatabase } from "./support.ts";

const SCHEMA = "disjoint";
const kindType = new TypeName("vehicle_kind", { schema: SCHEMA });

type Options = Record<string, unknown>;

function vehicle(kind: Options = {}) {
  return model("vehicle", {
    schema: SCHEMA,
    columns: {
      id: column("i32", { primaryKey: true, generated: "byDefault" }),
      kind: column(kindType, { values: ["car", "truck", "bike"], ...kind }),
      name: column("text"),
    },
  });
}

function car(key: Options = {}) {
  return model("car", {
    schema: SCHEMA,
    columns: {
      id: column("i32", { primaryKey: true, ...key }),
      seats: column("i16"),
      color: column(new TypeName("color", { schema: SCHEMA }), { nullable: true, values: ["red", "blue"] }),
    },
  });
}

function truck() {
  return model("truck", {
    schema: SCHEMA,
    columns: {
      vehicleId: column("i32", { name: "vehicle_id", primaryKey: true }),
      axles: column("i16"),
      payloadKg: column("i32", { name: "payload" }),
    },
  });
}

function vehicles<B extends Model<any> = ReturnType<typeof vehicle>, C extends Model<any> = ReturnType<typeof car>>(
  base: B = vehicle() as unknown as B,
  carModel: C = car() as unknown as C,
) {
  return disjoint("vehicle", base, { car: carModel, truck: truck(), bike: null }, { schemaName: SCHEMA });
}

const READ = [
  'SELECT "vehicle"."id" AS "s0_id", CAST("vehicle"."kind" AS text) AS "s0_kind",',
  '"vehicle"."name" AS "s0_name", "car"."id" AS "s1_id", "car"."seats" AS "s1_seats",',
  'CAST("car"."color" AS text) AS "s1_color", "truck"."vehicle_id" AS "s2_vehicle_id",',
  '"truck"."axles" AS "s2_axles", "truck"."payload" AS "s2_payload"',
  'FROM "disjoint"."vehicle"',
  'LEFT JOIN "disjoint"."car" ON "vehicle"."id" = "car"."id"',
  'LEFT JOIN "disjoint"."truck" ON "vehicle"."id" = "truck"."vehicle_id"',
].join(" ");

function refused(pattern?: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
    if (pattern) assert.match(error.message, pattern);
    return true;
  };
}

function undecodable(pattern?: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof DecodeError, `expected a DecodeError, got ${error}`);
    if (pattern) assert.match(error.message, pattern);
    return true;
  };
}

// [spec:pgorm:req:bindings.disjoint+1/test]    a disjoint model declared over models describes itself under the declared keys, and builds the statements pgorm's shape builds: the read, its narrowing by key, kind, kinds and a kind's condition, and an insert
test("a declaration builds the shape's statements", () => {
  const vehicleModel = vehicles();
  assert.deepStrictEqual(vehicleModel.describe(), {
    name: "vehicle",
    base: "vehicle",
    kindField: "kind",
    kindType: { schema: SCHEMA, name: "vehicle_kind" },
    kinds: [
      { value: "car", model: "car" },
      { value: "truck", model: "truck" },
      { value: "bike", model: null },
    ],
  });
  assert.equal(vehicleModel.find().inspect().sql, READ);
  assert.equal(vehicleModel.find().inspect({ terminal: "one" }).sql, `${READ} LIMIT $1`);
  const keyed = vehicleModel.findByKey({ id: 7 }).inspect();
  assert.equal(keyed.sql, `${READ} WHERE "vehicle"."id" = $1`);
  assert.equal(keyed.values.length, 1);
  assert.match(
    vehicleModel.find().kind("car").inspect().sql,
    /WHERE "vehicle"\."kind" = CAST\(\$1::text AS disjoint\.vehicle_kind\)$/,
  );
  assert.match(
    vehicleModel.find().kinds(["truck", "bike"]).inspect().sql,
    /WHERE "vehicle"\."kind" IN \(CAST\(\$1::text AS disjoint\.vehicle_kind\), CAST\(\$2::text AS disjoint\.vehicle_kind\)\)$/,
  );
  const carModel = car();
  const roomy = disjoint("vehicle", vehicle(), { car: carModel, truck: truck(), bike: null }, { schemaName: SCHEMA })
    .find()
    .filterKind("car", carModel.col("seats").gte(5))
    .inspect();
  assert.match(roomy.sql, /WHERE "vehicle"\."kind" = CAST\(\$1::text AS disjoint\.vehicle_kind\) AND "car"\."seats" >= \$2$/);
  assert.equal(
    vehicleModel.insert({ name: "mini" }, "car", { seats: 4 }).inspect().sql,
    [
      'WITH "vehicle" AS (INSERT INTO "disjoint"."vehicle" ("kind", "name")',
      "VALUES (CAST($1::text AS disjoint.vehicle_kind), $2)",
      'RETURNING "vehicle"."id", "vehicle"."kind", "vehicle"."name") ,',
      '"car" AS (INSERT INTO "disjoint"."car" ("id", "seats")',
      'SELECT "vehicle"."id", $3 FROM "vehicle"',
      'RETURNING "car"."id", "car"."seats", "car"."color")',
      READ.replace('FROM "disjoint"."vehicle"', 'FROM "vehicle"').replace('LEFT JOIN "disjoint"."car"', 'LEFT JOIN "car"'),
    ].join(" "),
  );
});

// [spec:pgorm:req:bindings.disjoint+1/test]    what the Rust declaration refuses is a ConstructionError thrown as the model is declared: no kinds, one model for two kinds, a kind model keyed by other types, a kind field missing, nullable, generated or of another type, and a kind model whose key the database fills; an argument of the wrong type is a TypeError
test("a declaration refuses what Rust refuses", () => {
  const cases: Array<[string, () => unknown, RegExp?]> = [
    ["no kinds", () => disjoint("vehicle", vehicle(), {}, { schemaName: SCHEMA })],
    ["two kinds one model", () => disjoint("vehicle", vehicle(), { car: car(), truck: car(), bike: null }, { schemaName: SCHEMA })],
    ["key types", () => vehicles(vehicle(), model("car", { schema: SCHEMA, columns: { id: column("i64", { primaryKey: true }) } }))],
    ["missing kind field", () => disjoint("vehicle", vehicle(), { car: car() }, { kind: "category", schemaName: SCHEMA })],
    ["nullable kind field", () => vehicles(vehicle({ nullable: true })), /the kind column `disjoint\.vehicle`\.`kind` is nullable/],
    ["generated kind field", () => vehicles(vehicle({ generated: "always" }))],
    [
      "another type",
      () => disjoint("vehicle", vehicle(), { car: car(), truck: truck(), bike: null }),
      /is `disjoint\.vehicle_kind` \('car', 'truck', 'bike'\), not the kind type `vehicle_kind`/,
    ],
    ["values unlisted", () => disjoint("vehicle", vehicle(), { car: car(), boat: null, bike: null }, { schemaName: SCHEMA })],
    ["generated kind key", () => vehicles(vehicle(), car({ generated: "byDefault" })), /the key of the kind `car`, is an identity/],
    ["defaulted kind key", () => vehicles(vehicle(), car({ default: true }))],
    ["aliased base", () => vehicles(vehicle().as("v"))],
  ];
  for (const [label, declare, pattern] of cases) {
    assert.throws(declare, refused(pattern), label);
  }
  for (const wrong of [
    // @ts-expect-error: a base is a Model
    () => disjoint("vehicle", "vehicle", { bike: null }),
    // @ts-expect-error: kinds are an object of models
    () => disjoint("vehicle", vehicle(), [["bike", null]]),
    // @ts-expect-error: a name is a string
    () => disjoint(1, vehicle(), { bike: null }),
    // @ts-expect-error: a kind's model is a Model or null
    () => disjoint("vehicle", vehicle(), { bike: "car" }),
    // @ts-expect-error: kindField is no option
    () => disjoint("vehicle", vehicle(), { bike: null }, { kindField: "kind" }),
  ]) {
    assert.throws(wrong, TypeError);
  }
});

// [spec:pgorm:req:bindings.disjoint+1/test]    a write refuses, as a ConstructionError before anything is sent, the kind field given, a kind row's key given, values for a kind without a table, a kind the model lacks and a field it lacks
test("writes refuse before sending", () => {
  const vehicleModel = vehicles();
  for (const write of [
    () => vehicleModel.insert({ kind: "car", name: "x" }, "car", { seats: 1 }),
    () => vehicleModel.insert({ name: "x" }, "car", { id: 1, seats: 1 }),
    // @ts-expect-error: a kind without a table takes no values
    () => vehicleModel.insert({ name: "x" }, "bike", { seats: 1 }),
    // @ts-expect-error: boat is no kind
    () => vehicleModel.insert({ name: "x" }, "boat"),
    // @ts-expect-error: wheels is no field
    () => vehicleModel.insert({ wheels: 3 }, "bike"),
    () => vehicleModel.update({ name: "x" }, "bike"),
    () => vehicleModel.update({ id: 1, kind: "car" }, "car"),
    () => vehicleModel.update({ id: 1 }, "car", { id: 2 }),
    () => vehicleModel.changeKind({ id: 1 }, "truck", { vehicleId: 1 }),
    // @ts-expect-error: boat is no kind
    () => vehicleModel.find().kind("boat"),
  ]) {
    assert.throws(write, refused());
  }
});

let database: { pool: Pool; drop(): Promise<void> } | undefined;

function pool(): Pool {
  if (!database) throw new Error("the scratch database was not created");
  return database.pool;
}

before(async () => {
  const scratch = await scratchDatabase("pgorm_napi_disjoint");
  const opened = new Pool(scratch.dsn, { maxSize: 4 });
  database = {
    pool: opened,
    async drop() {
      await opened.close();
      await scratch.drop();
    },
  };
});

beforeEach(async () => {
  await pool().execute(`DROP SCHEMA IF EXISTS ${SCHEMA} CASCADE`);
  await pool().execute(`CREATE SCHEMA ${SCHEMA}`);
  await pool().execute(`CREATE TYPE ${SCHEMA}.color AS ENUM ('red', 'blue')`);
  for (const statement of createDisjoint(vehicles())) await pool().execute(statement);
});

after(async () => {
  await database?.drop();
});

async function seed(vehicleModel = vehicles()) {
  const mini = await vehicleModel.insert({ name: "mini" }, "car", { seats: 4, color: "red" }).returningModel(pool());
  const lorry = await vehicleModel.insert({ name: "lorry" }, "truck", { axles: 3, payloadKg: 9000 }).returningModel(pool());
  const bmx = await vehicleModel.insert({ name: "bmx" }, "bike").returningModel(pool());
  return [mini, lorry, bmx];
}

// [spec:pgorm:req:bindings.disjoint+1/test]    the schema createDisjoint gives runs, and holds every row to its kind: a row inserted by hand for a kind with a table but no row is refused by the server's foreign key
test("the schema runs and holds rows to kinds", async () => {
  const statements = createDisjoint(vehicles());
  assert.deepStrictEqual(statements.map((statement) => statement.constructor.name), [
    "CreateType",
    "CreateTable",
    "CreateTable",
    "CreateTable",
    "AlterTable",
  ]);
  assert.ok(statements.every((statement) => statement instanceof DDL));
  await assert.rejects(
    pool().execute(`INSERT INTO ${SCHEMA}.vehicle (kind, name) VALUES ('car', 'ghost')`),
    (error: unknown) => {
      assert.ok(error instanceof ForeignKeyViolationError);
      assert.equal(error.sqlstate, "23503");
      return true;
    },
  );
  // @ts-expect-error: a model is no disjoint model
  assert.throws(() => createDisjoint(vehicle()), TypeError);
  const undeclared = disjoint(
    "vehicle",
    model("vehicle", {
      schema: SCHEMA,
      columns: {
        id: column("i32", { primaryKey: true, default: true }),
        kind: column(kindType, { values: ["car", "truck", "bike"] }),
      },
    }),
    { car: car(), truck: truck(), bike: null },
    { schemaName: SCHEMA },
  );
  assert.throws(() => createDisjoint(undeclared), refused(/has a default its description does not state/));
});

// [spec:pgorm:req:bindings.disjoint+1/test]    an insert of each kind returns what a read of its row gives, each row the base's record, the kind and the kind's record or null, an enum column of a kind's table read as its enum; reads narrow by key, kind, kinds and a kind's condition, order, slice, count, page, stream and read through a keyset cursor
test("inserts and reads", async () => {
  const carModel = car();
  const vehicleModel = vehicles(vehicle(), carModel);
  const [mini, lorry, bmx] = await seed(vehicleModel);
  assert.deepStrictEqual(mini, [{ id: 1, kind: "car", name: "mini" }, "car", { id: 1, seats: 4, color: "red" }]);
  assert.deepStrictEqual(lorry, [{ id: 2, kind: "truck", name: "lorry" }, "truck", { vehicleId: 2, axles: 3, payloadKg: 9000 }]);
  assert.deepStrictEqual(bmx, [{ id: 3, kind: "bike", name: "bmx" }, "bike", null]);
  const everything = [mini, lorry, bmx];
  const ordered = vehicleModel.find().orderBy(vehicleModel.base.col("id").asc());
  assert.deepStrictEqual(await ordered.all(pool()), everything);
  assert.deepStrictEqual(await vehicleModel.findByKey({ id: 2 }).one(pool()), lorry);
  await assert.rejects(vehicleModel.findByKey({ id: 99 }).one(pool()), undecodable());
  assert.deepStrictEqual(await ordered.kind("truck").all(pool()), [lorry]);
  assert.deepStrictEqual(await ordered.kinds(["car", "bike"]).all(pool()), [mini, bmx]);
  assert.deepStrictEqual(await ordered.filterKind("car", carModel.col("seats").isNull()).all(pool()), []);
  assert.deepStrictEqual(await ordered.where(vehicleModel.base.col("name").eq("bmx")).all(pool()), [bmx]);
  assert.deepStrictEqual(await ordered.limit(1).offset(1).all(pool()), [lorry]);
  assert.equal(await ordered.count(pool()), 3);
  const pages = ordered.paginate(2);
  assert.deepStrictEqual(await pages.numItemsAndPages(pool()), [3, 2]);
  const read = [];
  for await (const page of pages.pages(pool())) read.push(page);
  assert.deepStrictEqual(read, [[mini, lorry], [bmx]]);
  const streamed = [];
  for await (const row of ordered.stream(pool())) streamed.push(row);
  assert.deepStrictEqual(streamed, everything);
  await pool().connection(async (connection) => {
    const onConnection = [];
    for await (const row of ordered.stream(connection)) onConnection.push(row);
    assert.deepStrictEqual(onConnection, everything);
  });
  const cursor = vehicleModel.find().cursor("id");
  assert.deepStrictEqual(await cursor.after(1).first(1).all(pool()), [lorry]);
  assert.deepStrictEqual(await cursor.last(2).all(pool()), [lorry, bmx]);
  assert.deepStrictEqual(await cursor.desc().after(3).all(pool()), [lorry, mini]);
  assert.throws(() => cursor.after(1, 2), refused());
});

// [spec:pgorm:req:bindings.disjoint+1/test]    an update writes the base, the kind's row, both or neither, a row of another kind or a key no row has being a DecodeError that changes nothing; a change of kind moves the row between tables and to the kind without one, to its own kind being a DecodeError; a server's refusal keeps its class
test("updates and changes of kind", async () => {
  const vehicleModel = vehicles();
  const [, , bmx] = await seed(vehicleModel);
  assert.deepStrictEqual(
    await vehicleModel.update({ id: 1, name: "cooper" }, "car", { seats: 5 }).returningModel(pool()),
    [{ id: 1, kind: "car", name: "cooper" }, "car", { id: 1, seats: 5, color: "red" }],
  );
  const onlyRow = await vehicleModel.update({ id: 2 }, "truck", { axles: 4 }).returningModel(pool());
  assert.equal(onlyRow[1], "truck");
  if (onlyRow[1] === "truck") assert.equal(onlyRow[2].axles, 4);
  assert.deepStrictEqual(await vehicleModel.update({ id: 3 }, "bike").returningModel(pool()), bmx);
  for (const miss of [
    vehicleModel.update({ id: 1, name: "x" }, "truck", { axles: 9 }),
    vehicleModel.update({ id: 99, name: "x" }, "bike"),
    vehicleModel.changeKind({ id: 1 }, "car"),
    vehicleModel.changeKind({ id: 99 }, "bike"),
  ]) {
    await assert.rejects(miss.returningModel(pool()), undecodable());
  }
  assert.deepStrictEqual(
    await vehicleModel.changeKind({ id: 1 }, "truck", { axles: 2, payloadKg: 500 }).returningModel(pool()),
    [{ id: 1, kind: "truck", name: "cooper" }, "truck", { vehicleId: 1, axles: 2, payloadKg: 500 }],
  );
  assert.deepStrictEqual(
    await vehicleModel.changeKind({ id: 1 }, "bike").returningModel(pool()),
    [{ id: 1, kind: "bike", name: "cooper" }, "bike", null],
  );
  assert.deepStrictEqual(await pool().query(`SELECT count(*)::int4 AS n FROM ${SCHEMA}.car`), [{ n: 0 }]);
  await assert.rejects(vehicleModel.insert({ name: "seatless" }, "car").returningModel(pool()), (error: unknown) => {
    assert.ok(error instanceof NotNullViolationError);
    return true;
  });
  assert.equal(await vehicleModel.find().count(pool()), 3);
});

// [spec:pgorm:req:bindings.disjoint+1/test]    rows of tables written without the constraints, a car with no car row and a bike with a truck row, are each a DecodeError naming the base table, the key and the kinds, never a row read without its row
test("inconsistent rows are decode errors", async () => {
  await pool().execute(`DROP SCHEMA ${SCHEMA} CASCADE`);
  await pool().execute(`CREATE SCHEMA ${SCHEMA}`);
  for (const sql of [
    `CREATE TYPE ${SCHEMA}.color AS ENUM ('red', 'blue')`,
    `CREATE TYPE ${SCHEMA}.vehicle_kind AS ENUM ('car', 'truck', 'bike')`,
    `CREATE TABLE ${SCHEMA}.vehicle (id integer PRIMARY KEY, kind ${SCHEMA}.vehicle_kind NOT NULL, name text NOT NULL)`,
    `CREATE TABLE ${SCHEMA}.car (id integer PRIMARY KEY, seats smallint NOT NULL, color ${SCHEMA}.color)`,
    `CREATE TABLE ${SCHEMA}.truck (vehicle_id integer PRIMARY KEY, axles smallint NOT NULL, payload integer NOT NULL)`,
    `INSERT INTO ${SCHEMA}.vehicle VALUES (1, 'car', 'ghost'), (2, 'bike', 'hauler')`,
    `INSERT INTO ${SCHEMA}.truck VALUES (2, 2, 300)`,
  ]) {
    await pool().execute(sql);
  }
  const vehicleModel = vehicles();
  await assert.rejects(
    vehicleModel.findByKey({ id: 1 }).one(pool()),
    undecodable(/the `disjoint\.vehicle` row whose key is \(`id` = 1\) is of the kind `car` but has no row in `disjoint\.car`/),
  );
  await assert.rejects(
    vehicleModel.findByKey({ id: 2 }).one(pool()),
    undecodable(/a row in `disjoint\.truck`, the table of the kind `truck`/),
  );
  await assert.rejects(vehicleModel.find().all(pool()), undecodable());
});
