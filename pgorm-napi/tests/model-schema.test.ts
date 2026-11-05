// A declared model's own DDL, written by pgorm's Schema from the TableShape
// the model describes, as an entity's is: its table with its keys, its enum
// types, indexes and comments, refused where the declaration cannot say it.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import {
  call,
  col,
  column,
  ConstructionError,
  createDisjoint,
  DataType,
  Decimal,
  disjoint,
  EntitySchema,
  ForeignKeyViolationError,
  fromEntity,
  fromModel,
  model,
  type Pool,
  TypeName,
  Value,
} from "../lib/index.js";
import { mistyped, scratchDatabase } from "./support.ts";
import { Pool as PoolClass } from "../lib/index.js";

let database: { pool: Pool; drop(): Promise<void> } | undefined;

before(async () => {
  const scratch = await scratchDatabase("pgorm_napi_model_schema");
  const pool = new PoolClass(scratch.dsn, { maxSize: 2 });
  database = {
    pool,
    async drop() {
      await pool.close();
      await scratch.drop();
    },
  };
});

after(async () => {
  await database?.drop();
});

function pool(): Pool {
  if (!database) throw new Error("the scratch database was not created");
  return database.pool;
}

function refused(pattern: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
    assert.match(error.message, pattern);
    return true;
  };
}

const money = new DataType("numeric", { precision: 10, scale: 2 });

function bakery() {
  return model("bakery", {
    schema: "model_ddl",
    columns: {
      id: column("i32", { primaryKey: true, autoIncrement: true }),
      name: column("text", { columnType: "varchar" }),
    },
  });
}

function cake(owner: ReturnType<typeof bakery>) {
  const Cake = model("cake", {
    schema: "model_ddl",
    comment: "one row per cake",
    renameAll: "snake_case",
    uniqueKeys: [{ fields: ["bakeryId", "name"], name: "one_name_per_bakery" }],
    columns: {
      id: column("i32", { primaryKey: true, autoIncrement: true }),
      name: column("text", { columnType: new DataType("varchar", { length: 40 }), indexed: true }),
      flavor: column(new TypeName("flavor", { schema: "model_ddl" }), { values: ["lemon", "plain"], defaultValue: "plain" }),
      price: column("decimal", { columnType: money, comment: "in euros" }),
      doubled: column("decimal", { columnType: money, generatedStored: col("price").mul(2) }),
      code: column("uuid", { defaultExpr: call("uuidv7") }),
      tags: column("text", { array: true, columnType: "varchar", nullable: true }),
      bakeryId: column("i32"),
    },
  });
  return [Cake, Cake.belongsTo(owner, { from: "bakeryId", to: "id", onDelete: "cascade" })] as const;
}

// [spec:pgorm:req:bindings.schema-from-model/test]    every part of a declaration is written as an entity's
// [spec:pgorm:req:bindings.relations+2/test]    a belongsTo relation's actions reach its foreign key
test("every option of a declaration is written", () => {
  const [Cake, toBakery] = cake(bakery());
  const schema = fromModel(Cake, { relations: [toBakery] });
  assert.ok(schema instanceof EntitySchema);
  assert.equal(
    schema.table.inspect().sql,
    `CREATE TABLE "model_ddl"."cake" ( "id" serial NOT NULL, "name" varchar(40) NOT NULL, ` +
      `"flavor" model_ddl.flavor NOT NULL DEFAULT CAST('plain' AS model_ddl.flavor), ` +
      `"price" decimal(10, 2) NOT NULL, ` +
      `"doubled" decimal(10, 2) NOT NULL GENERATED ALWAYS AS ("price" * 2) STORED, ` +
      `"code" uuid NOT NULL DEFAULT UUIDV7(), "tags" varchar[], ` +
      `"bakery_id" integer NOT NULL, PRIMARY KEY ("id"), ` +
      `CONSTRAINT "one_name_per_bakery" UNIQUE ("bakery_id", "name"), ` +
      `CONSTRAINT "fk-cake-bakery_id" FOREIGN KEY ("bakery_id") ` +
      `REFERENCES "model_ddl"."bakery" ("id") ON DELETE CASCADE )`,
  );
  assert.deepEqual(schema.enums.map((statement) => statement.inspect().sql), [
    `CREATE TYPE "model_ddl"."flavor" AS ENUM ('lemon', 'plain')`,
  ]);
  assert.deepEqual(schema.indexes.map((statement) => statement.inspect().sql), [
    `CREATE INDEX "idx-cake-name" ON "model_ddl"."cake" ("name")`,
  ]);
  assert.deepEqual(schema.comments.map((statement) => statement.inspect().sql), [
    `COMMENT ON TABLE "model_ddl"."cake" IS 'one row per cake'`,
    `COMMENT ON COLUMN "model_ddl"."cake"."price" IS 'in euros'`,
  ]);
  assert.equal(Cake.columns.bakeryId.name, null);
  assert.equal(Cake.describe().fields.bakeryId!.column, "bakery_id");
  assert.equal(Cake.comment, "one row per cake");
  assert.equal(Cake.describe().fields.doubled!.generated, "always");
  assert.equal(toBakery.onDelete, "cascade");
  assert.equal(toBakery.fkName, null);
  assert.ok(Cake.columns.flavor.defaultValue instanceof Value);
});

// [spec:pgorm:req:bindings.schema-from-model/test]    keys: composite, identity, temporal, named foreign keys
test("keys are written as an entity's", () => {
  const Booking = model("booking", {
    columns: {
      room: column("i32", { primaryKey: true }),
      id: column("i64", { primaryKey: true, generated: "always" }),
      during: column("tstzrange", { primaryKey: true, withoutOverlaps: true }),
    },
  });
  assert.equal(
    fromModel(Booking).table.inspect().sql,
    `CREATE TABLE "booking" ( "room" integer NOT NULL, ` +
      `"id" bigint NOT NULL GENERATED ALWAYS AS IDENTITY, "during" tstzrange NOT NULL, ` +
      `CONSTRAINT "pk-booking" PRIMARY KEY ("room", "id", "during" WITHOUT OVERLAPS) )`,
  );
  const Room = model("room", { columns: { id: column("i32", { primaryKey: true }) } });
  const named = Booking.belongsTo(Room, { from: "room", to: "id", onUpdate: "restrict", fkName: "booking_room" });
  assert.ok(
    fromModel(Booking, { relations: [named] }).table.inspect().sql.includes(
      `CONSTRAINT "booking_room" FOREIGN KEY ("room") REFERENCES "room" ("id") ON UPDATE RESTRICT`,
    ),
  );
  const Log = model("log", { columns: { line: column("text"), at: column("date", { nullable: true }) } });
  assert.equal(fromModel(Log).table.inspect().sql, `CREATE TABLE "log" ( "line" text NOT NULL, "at" date )`);
});

// [spec:pgorm:req:bindings.schema-from-model/test]    renameAll names columns by the derive's conversion
test("renameAll is the derive's rename_all", () => {
  const styles = [
    ["snake_case", "bake_minutes"],
    ["PascalCase", "BakeMinutes"],
    ["kebab-case", "bake-minutes"],
    ["SCREAMING_SNAKE_CASE", "BAKE_MINUTES"],
    ["SCREAMING-KEBAB-CASE", "BAKE-MINUTES"],
    ["title_case", "Bake Minutes"],
    ["mixed_case", "bakeMinutes"],
  ] as const;
  for (const [style, name] of styles) {
    const Recipe = model("recipe", {
      renameAll: style,
      columns: { bakeMinutes: column("i32"), kept: column("text", { name: "as_is" }) },
    });
    assert.equal(Recipe.describe().fields.bakeMinutes!.column, name, style);
    assert.equal(Recipe.describe().fields.kept!.column, "as_is");
  }
  assert.throws(
    // @ts-expect-error: no case style
    () => model("recipe", { renameAll: "Camel", columns: { id: column("i32") } }),
    mistyped(/is no case style/),
  );
});

// [spec:pgorm:req:bindings.schema-from-model/test]    what the options cannot say is refused as the model is declared
test("what the options cannot say is refused as the model is declared", () => {
  const refusals = [
    () => column(new TypeName("flavor", { schema: "public" }), { columnType: "text" }),
    () => model("t", { columns: { c: column("text", { array: true, columnType: new DataType("text").array() }) } }),
    () => column("i32", { default: true, defaultValue: 1 }),
    () => column("i32", { defaultValue: 1, defaultExpr: col("x") }),
    () => column("i32", { generated: "byDefault", generatedStored: col("x") }),
    () => column("i32", { primaryKey: true, autoIncrement: true, generated: "always" }),
    () => column("text", { primaryKey: true, autoIncrement: true }),
    () => column("i32", { autoIncrement: true }),
    () => column("i32", { primaryKey: true, withoutOverlaps: true }),
    () => column("i32", { defaultValue: Value.null("i32") }),
    () => column("i32", { defaultValue: "one" }),
    () =>
      model("t", {
        columns: {
          a: column("i32", { primaryKey: true, autoIncrement: true }),
          b: column("i32", { primaryKey: true }),
        },
      }),
    () =>
      model("t", {
        columns: {
          a: column("tstzrange", { primaryKey: true, withoutOverlaps: true }),
          b: column("i32", { primaryKey: true }),
        },
      }),
    () => model("t", { columns: { a: column("tstzrange", { primaryKey: true, withoutOverlaps: true }) } }),
    () =>
      model("t", {
        columns: { a: column("i32", { generatedVirtual: col("b") }), b: column("i32") },
        uniqueKeys: [{ fields: "a" }],
      }),
  ];
  for (const [index, refusal] of refusals.entries()) {
    assert.throws(refusal, (error: unknown) => error instanceof ConstructionError, `refusal ${index}`);
  }
  // @ts-expect-error: defaultExpr takes an Expr, never a plain number
  assert.throws(() => column("i32", { defaultExpr: 1 }), mistyped(/defaultExpr is an expression/));
  // @ts-expect-error: a generation expression is an expression
  assert.throws(() => column("i32", { generatedStored: "price" }), mistyped(/generatedStored is an expression/));
  assert.throws(() => column("i32", { defaultValue: col("x") }), mistyped(/defaultValue is a value/));
  // @ts-expect-error: a comment is a string
  assert.throws(() => column("i32", { comment: 3 }), mistyped(/comment is a string/));
  // @ts-expect-error: a comment is a string
  assert.throws(() => model("t", { comment: 3, columns: { a: column("i32") } }), mistyped(/comment is a string/));
});

// [spec:pgorm:req:bindings.schema-from-model/test]    the schema refuses relations it holds no key of, and unstated defaults
// [spec:pgorm:req:bindings.relations+2/test]    only belongsTo takes a foreign key's options
test("the schema refuses what it holds no key of", () => {
  const owner = bakery();
  const [Cake, toBakery] = cake(owner);
  const cakes = owner.hasMany(Cake, { from: "id", to: "bakeryId" });
  assert.throws(() => fromModel(Cake, { relations: [cakes] }), refused(/holds no foreign key/));
  assert.throws(() => fromModel(owner, { relations: [toBakery] }), refused(/starts at cake, not at bakery/));
  assert.throws(() => fromModel(Cake.as("c")), refused(/created under its name/));
  // @ts-expect-error: a model is no relation
  assert.throws(() => fromModel(Cake, { relations: [Cake] }), mistyped(/belongsTo relations/));
  // @ts-expect-error: a model is no registered entity
  assert.throws(() => fromEntity(Cake), mistyped(/fromModel/));
  const unstated = model("t", { columns: { id: column("i32", { primaryKey: true, default: true }) } });
  assert.throws(() => fromModel(unstated), refused(/has a default its description does not state/));
  assert.throws(
    // @ts-expect-error: hasMany holds no foreign key
    () => owner.hasMany(Cake, { from: "id", to: "bakeryId", onDelete: "cascade" }),
    mistyped(/only it takes onDelete/),
  );
  assert.throws(
    // @ts-expect-error: no such action
    () => Cake.belongsTo(owner, { from: "bakeryId", to: "id", onDelete: "drop" }),
    mistyped(/onDelete is one of/),
  );
});

// [spec:pgorm:req:bindings.schema-from-model/test]    a disjoint model's tables take the same column lowering
test("a disjoint model's tables take column types and stated defaults", () => {
  const Vehicle = model("vehicle", {
    columns: {
      id: column("i32", { primaryKey: true }),
      kind: column(new TypeName("vehicle_kind", { schema: "public" }), { values: ["car"] }),
      label: column("text", { columnType: "varchar", defaultValue: "unnamed" }),
    },
  });
  const Car = model("car", { columns: { id: column("i32", { primaryKey: true }), doors: column("i16") } });
  const vehicles = disjoint("vehicle", Vehicle, { car: Car }, { schemaName: "public" });
  const tables = createDisjoint(vehicles).map((statement) => statement.inspect().sql);
  assert.ok(tables[1]!.includes(`"label" varchar NOT NULL DEFAULT 'unnamed'`), tables[1]);
});

// [spec:pgorm:req:bindings.schema-from-model/test]    the statements run, and the tables hold what the models declare
test("a model's schema creates its table", async () => {
  const db = pool();
  await db.execute(`CREATE SCHEMA model_ddl`);
  const owner = bakery();
  const [Cake, toBakery] = cake(owner);
  const cakeSchema = fromModel(Cake, { relations: [toBakery] });
  for (const statement of [
    ...cakeSchema.enums,
    fromModel(owner).table,
    cakeSchema.table,
    ...cakeSchema.indexes,
    ...cakeSchema.comments,
  ]) {
    await db.execute(statement);
  }
  const made = await owner.insert({ name: "Crumb" }).returning().one(db);
  const row = await Cake.insert({ name: "Lemon Drizzle", price: new Decimal("3.50"), bakeryId: made.id }).returning().one(db);
  assert.equal(row.flavor, "plain");
  assert.equal(row.doubled.toString(), "7.00");
  await assert.rejects(
    Cake.insert({ name: "Orphan", price: new Decimal("1"), bakeryId: 99 }).execute(db),
    (error: unknown) => error instanceof ForeignKeyViolationError,
  );
  await owner.delete().allRows().execute(db);
  assert.equal(await Cake.find().count(db), 0);
  const [comment] = await db.query(`SELECT obj_description('model_ddl.cake'::regclass, 'pg_class') AS c`);
  assert.equal(comment!.c, "one row per cake");
});
