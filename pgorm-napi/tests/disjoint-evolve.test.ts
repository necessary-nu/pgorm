// A disjoint model's kind changes, in either runtime, written by pgorm's
// DisjointShape from the declaration as it will be: the steps and gaps each
// change gives, its refusals, and each change run on a live server.

import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import {
  addDisjointKind,
  addDisjointKindRecreating,
  addDisjointKindTable,
  CheckViolationError,
  column,
  ConstructionError,
  createDisjoint,
  DDL,
  disjoint,
  dropTable,
  type Disjoint,
  type DisjointChange,
  type DisjointStep,
  ForeignKeyViolationError,
  model,
  type Model,
  Pool,
  removeDisjointKind,
  removeDisjointKindTable,
  Table,
  TypeName,
} from "../lib/index.js";
import { scratchDatabase } from "./support.ts";

const SCHEMA = "disjoint_evolve";
const kindType = new TypeName("vehicle_kind", { schema: SCHEMA });

type Columns = Record<string, ReturnType<typeof column>>;

function table(name: string, columns: Columns) {
  return model(name, { schema: SCHEMA, columns: { id: column("i32", { primaryKey: true }), ...columns } });
}

function vehicle(values: string[]) {
  return model("vehicle", {
    schema: SCHEMA,
    columns: {
      id: column("i32", { primaryKey: true, generated: "byDefault" }),
      kind: column(kindType, { values }),
      name: column("text"),
    },
  });
}

/**
 * The declaration whose kinds are `kinds`, in order, each a kind table's
 * columns or null, its kinds typed as models since they are chosen at run
 * time.
 */
function vehicles(kinds: Record<string, Columns | null>): Disjoint<ReturnType<typeof vehicle>, Record<string, Model<any>>> {
  const tables: Record<string, Model<any> | null> = {};
  for (const [value, columns] of Object.entries(kinds)) tables[value] = columns === null ? null : table(value, columns);
  return disjoint("vehicle", vehicle(Object.keys(kinds)), tables as Record<string, Model<any>>, { schemaName: SCHEMA });
}

const CAR = { seats: column("i16") };
const TRUCK = { axles: column("i16") };
const VAN = { doors: column("i16") };
const BIKE = { gears: column("i16") };

function refused(pattern?: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
    if (pattern) assert.match(error.message, pattern);
    return true;
  };
}

// [spec:pgorm:req:bindings.disjoint-evolve/test]    each change gives its steps as pgorm's shape writes them, frozen data sending nothing: a kind with a table added in two steps, a kind table added with a gap naming the kind and its table, and a kind table dropped in one step
test("each change gives the shape's steps", () => {
  const added = addDisjointKind(vehicles({ car: CAR, van: VAN, bike: null }), "van");
  const tabled = addDisjointKindTable(vehicles({ car: CAR, bike: BIKE }), "bike");
  const dropped = removeDisjointKindTable(vehicles({ car: CAR, bike: null }), "bike", new Table("bike", { schema: SCHEMA }));
  assert.ok(Object.isFrozen(added) && Object.isFrozen(added.steps) && Object.isFrozen(added.steps[0]));
  assert.deepStrictEqual(added.steps[0]?.statements.map((statement) => statement.inspect().sql), [
    `ALTER TYPE "${SCHEMA}"."vehicle_kind" ADD VALUE 'van' AFTER 'car'`,
  ]);
  assert.deepStrictEqual(added.steps[1]?.statements.map((statement) => statement.constructor.name), [
    "CreateTable",
    "AlterTable",
  ]);
  assert.deepStrictEqual(added.steps.map((step) => step.gap), [null, null]);
  const gap = tabled.steps[0]?.gap;
  assert.equal(gap?.kind, "bike");
  assert.ok(gap && gap.table instanceof Table);
  assert.equal(dropTable(gap.table).inspect().sql, `DROP TABLE "${SCHEMA}"."bike"`);
  assert.equal(tabled.steps[1]?.gap, null);
  assert.match(tabled.steps[1]?.statements[0]?.inspect().sql ?? "", /VALIDATE CONSTRAINT/);
  assert.equal(dropped.steps.length, 1);
  assert.equal(dropped.steps[0]?.statements.at(-1)?.inspect().sql, `DROP TABLE "${SCHEMA}"."bike"`);
  assert.ok(added.steps.every((step) => step.statements.every((statement) => statement instanceof DDL)));
});

// [spec:pgorm:req:bindings.disjoint-evolve/test]    a change the declaration does not describe is a ConstructionError as the function is called, an unkinded model's a ConstructionError naming it read-only, and an argument of the wrong type a TypeError
test("each change refuses what the shape refuses", () => {
  const two = vehicles({ car: CAR, bike: null });
  assert.throws(() => addDisjointKind(two, "van"), refused(/`van` is no kind of the disjoint entity/));
  assert.throws(() => addDisjointKindRecreating(two, "van"), refused());
  assert.throws(() => removeDisjointKind(two, "bike"), refused(/declares the kind `bike`, so it is not removed/));
  assert.throws(() => addDisjointKindTable(two, "bike"), refused(/without a table/));
  assert.throws(() => removeDisjointKindTable(two, "car", "car"), refused(/so its table is not dropped/));
  const shop = model("shop", { schema: SCHEMA, columns: { id: column("i32", { primaryKey: true }) } });
  const unkinded = disjoint("shop", shop, { web: table("web", {}), dormant: null }, { unkinded: true });
  // @ts-expect-error: an unkinded model's kinds are not changed
  assert.throws(() => addDisjointKind(unkinded, "web"), refused(/read-only/));
  // @ts-expect-error: a model is no disjoint model
  assert.throws(() => addDisjointKind(two.base, "car"), TypeError);
  // @ts-expect-error: a kind's value is a string
  assert.throws(() => addDisjointKind(two, 1), TypeError);
  // @ts-expect-error: a table is named by a string or a Table
  assert.throws(() => removeDisjointKindTable(two, "bike", 1), TypeError);
});

let database: { pool: Pool; drop(): Promise<void> } | undefined;

function pool(): Pool {
  if (!database) throw new Error("the scratch database was not created");
  return database.pool;
}

before(async () => {
  const scratch = await scratchDatabase("pgorm_napi_disjoint_evolve");
  const opened = new Pool(scratch.dsn, { maxSize: 2 });
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
  pool().clearStatementCaches();
  const created = vehicles({ car: CAR, truck: TRUCK, bike: null });
  for (const statement of createDisjoint(created)) await pool().execute(statement);
  await created.insert({ name: "mini" }, "car", { seats: 4 }).returningModel(pool());
  await created.insert({ name: "lorry" }, "truck", { axles: 3 }).returningModel(pool());
  await created.insert({ name: "bmx" }, "bike").returningModel(pool());
});

after(async () => {
  await database?.drop();
});

async function runStep(step: DisjointStep) {
  await pool().transaction(async (tx) => {
    for (const statement of step.statements) await tx.execute(statement);
  });
  pool().clearStatementCaches();
}

async function runChange(change: DisjointChange) {
  for (const step of change.steps) await runStep(step);
}

async function kinds() {
  const [row] = await pool().query(`SELECT enum_range(NULL::${SCHEMA}.vehicle_kind)::text AS kinds`);
  return (row as { kinds: string }).kinds;
}

async function read(declared: ReturnType<typeof vehicles>) {
  const rows = await declared.find().orderBy(declared.base.col("id").asc()).all(pool());
  return rows.map(([base, kind, record]) => [(base as { name: string }).name, kind, record]);
}

// [spec:pgorm:req:bindings.disjoint-evolve/test]    a kind added in two steps and one added by recreating the kind type each run, the declaration as it will be then reading every row and writing the new kinds
test("kinds are added through both routes", async () => {
  const withVan = vehicles({ car: CAR, truck: TRUCK, van: VAN, bike: null });
  await runChange(addDisjointKind(withVan, "van"));
  await withVan.insert({ name: "kombi" }, "van", { doors: 3 }).returningModel(pool());
  const withBoat = vehicles({ car: CAR, truck: TRUCK, van: VAN, bike: null, boat: null });
  const change = addDisjointKindRecreating(withBoat, "boat");
  assert.equal(change.steps.length, 1);
  await runChange(change);
  await withBoat.insert({ name: "dinghy" }, "boat").returningModel(pool());
  assert.equal(await kinds(), "{car,truck,van,bike,boat}");
  assert.deepStrictEqual(await read(withBoat), [
    ["mini", "car", { id: 1, seats: 4 }],
    ["lorry", "truck", { id: 2, axles: 3 }],
    ["bmx", "bike", null],
    ["kombi", "van", { id: 4, doors: 3 }],
    ["dinghy", "boat", null],
  ]);
});

// [spec:pgorm:req:bindings.disjoint-evolve/test]    a kind table is added across its gap, the program writing the rows its base rows lack between the steps, and the second step refused while one is missing; a kind table dropped turns the kind into one with no table, its rows lost
test("kind tables are added and removed", async () => {
  const bikes = vehicles({ car: CAR, truck: TRUCK, bike: BIKE });
  const [first, second] = addDisjointKindTable(bikes, "bike").steps;
  assert.ok(first && second && first.gap);
  await runStep(first);
  await assert.rejects(runStep(second), ForeignKeyViolationError);
  await pool().execute(
    `INSERT INTO ${SCHEMA}.bike (id, gears) SELECT id, 21 FROM ${SCHEMA}.vehicle WHERE kind = '${first.gap.kind}'`,
  );
  await runStep(second);
  assert.deepStrictEqual((await read(bikes))[2], ["bmx", "bike", { id: 3, gears: 21 }]);
  const noTrucks = vehicles({ car: CAR, truck: null, bike: BIKE });
  await runChange(removeDisjointKindTable(noTrucks, "truck", new Table("truck", { schema: SCHEMA })));
  assert.deepStrictEqual((await read(noTrucks))[1], ["lorry", "truck", null]);
  assert.deepStrictEqual(
    await pool().query(`SELECT count(*)::int4 AS n FROM pg_tables WHERE schemaname = '${SCHEMA}' AND tablename = 'truck'`),
    [{ n: 0 }],
  );
});

// [spec:pgorm:req:bindings.disjoint-evolve/test]    a kind removed while a base row has it is refused by the server's check (23514) and changes nothing; once no row has it, the kind type is recreated without it
test("a kind is removed only once no row has it", async () => {
  const withoutBikes = vehicles({ car: CAR, truck: TRUCK });
  const change = removeDisjointKind(withoutBikes, "bike");
  await assert.rejects(runChange(change), (error: unknown) => {
    assert.ok(error instanceof CheckViolationError, `expected a CheckViolationError, got ${error}`);
    assert.equal(error.sqlstate, "23514");
    return true;
  });
  assert.equal(await kinds(), "{car,truck,bike}");
  await pool().execute(`DELETE FROM ${SCHEMA}.vehicle WHERE kind = 'bike'`);
  await runChange(change);
  assert.equal(await kinds(), "{car,truck}");
  assert.deepStrictEqual((await read(withoutBikes)).map(([, kind]) => kind), ["car", "truck"]);
});
