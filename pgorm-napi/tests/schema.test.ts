// Tables built from JavaScript against a live server, in either runtime: their
// columns, keys and constraints, what each means once PostgreSQL holds it,
// their alterations, and the shapes refused as a builder is called.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import * as pgorm from "../lib/index.js";
import {
  alterTable,
  col,
  ColumnDef,
  commentOnColumn,
  commentOnTable,
  ConstructionError,
  createExtension,
  createIndex,
  createSchema,
  createSequence,
  createTable,
  createType,
  DatabaseError,
  DataType,
  DDL,
  dropTable,
  Pool,
  renameColumn,
  renameConstraint,
  renameSchema,
  renameTable,
  type Statement,
  Table,
  truncateTable,
} from "../lib/index.js";
import { mistyped, scratchDatabase } from "./support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

before(async () => {
  database = await scratchDatabase("pgorm_napi_schema");
  shared = new Pool(database.dsn, { maxSize: 4 });
  await pool().execute("CREATE SCHEMA app");
  await pool().execute(createExtension("btree_gist", { ifNotExists: true }));
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

/**
 * Each constraint of `table` but a key column's implied `NOT NULL`, as
 * `[name, type, valid, enforced, noInherit]`; `noInherit` is a `CHECK`'s or a
 * `NOT NULL`'s alone, PostgreSQL recording every key as not inherited.
 */
async function constraints(table: string): Promise<unknown[]> {
  const rows = await pool().query(
    `SELECT conname, contype::text, convalidated, conenforced,
            CASE WHEN contype IN ('c', 'n') THEN connoinherit END AS connoinherit
     FROM pg_constraint c WHERE conrelid = $1::text::regclass AND NOT (contype = 'n' AND EXISTS (
       SELECT FROM pg_constraint k WHERE k.conrelid = c.conrelid AND k.contype = 'p' AND c.conkey <@ k.conkey))
     ORDER BY conname`,
    [table],
  );
  return rows.map((row) => [row.conname, row.contype, row.convalidated, row.conenforced, row.connoinherit]);
}

// [spec:pgorm:req:bindings.schema-tables/test]
test("a table is created with its identity, defaults, generated columns and collation, each meaning what it says", async () => {
  const visit = new Table("visit", { schema: "app" });
  await pool().execute(
    createTable(visit)
      .column(new ColumnDef("id", "bigint").identity("always", { startWith: 10, incrementBy: 5 }))
      .column(new ColumnDef("seats", "integer").notNull().default(2))
      .column(new ColumnDef("price", new DataType("numeric", { precision: 6, scale: 2 })).default(9.5))
      .column(new ColumnDef("doubled", "integer").generated(col("seats").mul(2), "stored"))
      .column(new ColumnDef("tripled", "integer").generated(col("seats").mul(3), "virtual"))
      .column(new ColumnDef("note", "text").collate("C"))
      .primaryKey("id"),
  );
  await pool().execute("INSERT INTO app.visit (seats, note) VALUES (DEFAULT, 'b'), (7, 'B')");
  const rows = await pool().query("SELECT id, seats, price::text, doubled, tripled FROM app.visit ORDER BY id");
  assert.deepStrictEqual(rows.map((row) => Object.values(row)), [[10n, 2, "9.50", 4, 6], [15n, 7, "9.50", 14, 21]]);
  const kinds = await pool().query(
    `SELECT attname, attgenerated::text, attidentity::text, collname FROM pg_attribute LEFT JOIN pg_collation c ON c.oid = attcollation
     WHERE attrelid = 'app.visit'::regclass AND attnum > 0 ORDER BY attnum`,
  );
  assert.deepStrictEqual(kinds.map((row) => [row.attname, row.attgenerated, row.attidentity, row.collname]), [
    ["id", "", "a", null],
    ["seats", "", "", null],
    ["price", "", "", null],
    ["doubled", "s", "", null],
    ["tripled", "v", "", null],
    ["note", "", "", "C"],
  ]);
  await assert.rejects(pool().execute("INSERT INTO app.visit (id, seats) VALUES (1, 1)"), rejectedWith("428C9"));
  await assert.rejects(pool().execute("INSERT INTO app.visit (seats) VALUES (NULL)"), rejectedWith("23502"));
});

// [spec:pgorm:req:bindings.schema-tables/test]
test("a column's NOT NULL is named and kept from inheriting tables, and a NOT ENFORCED CHECK holds no row", async () => {
  await pool().execute(
    createTable("parent")
      .column(new ColumnDef("a", "integer").notNull({ name: "a_present", noInherit: true }))
      .column(new ColumnDef("b", "integer").check(col("b").gt(0), { name: "b_positive", enforcement: "notEnforced" }))
      .check(col("a").lt(100), { name: "a_small", noInherit: true }),
  );
  assert.deepStrictEqual(await constraints("parent"), [
    ["a_present", "n", true, true, true],
    ["a_small", "c", true, true, true],
    ["b_positive", "c", false, false, false],
  ]);
  await pool().execute("INSERT INTO parent VALUES (1, -5)");
  await assert.rejects(pool().execute("INSERT INTO parent VALUES (NULL, 1)"), rejectedWith("23502"));
  await assert.rejects(pool().execute("INSERT INTO parent VALUES (500, 1)"), rejectedWith("23514"));
  await pool().execute("CREATE TABLE child () INHERITS (parent)");
  await pool().execute("INSERT INTO child VALUES (NULL, 1), (500, 1)");
});

// [spec:pgorm:req:bindings.schema-tables/test]
test("a key is the table's: one primary key, unique keys with NULLS NOT DISTINCT, INCLUDE and WITHOUT OVERLAPS", async () => {
  await pool().execute(
    createTable("room")
      .column(new ColumnDef("id", "integer"))
      .column(new ColumnDef("code", "text"))
      .column(new ColumnDef("wing", "text"))
      .column(new ColumnDef("open", "daterange").notNull())
      .primaryKey("code", { name: "replaced" })
      .primaryKey("id", { name: "room_key", withoutOverlaps: "open", include: ["wing"] })
      .unique("code", { nullsNotDistinct: true, name: "room_code" }),
  );
  assert.deepStrictEqual(
    (await constraints("room")).filter((row) => (row as string[])[1] !== "n"),
    [["room_code", "u", true, true, null], ["room_key", "p", true, true, null]],
  );
  await pool().execute("INSERT INTO room VALUES (1, NULL, 'east', '[2020-01-01,2020-02-01)')");
  await pool().execute("INSERT INTO room VALUES (1, 'b', 'east', '[2020-02-01,2020-03-01)')");
  await assert.rejects(
    pool().execute("INSERT INTO room VALUES (1, 'c', 'east', '[2020-01-15,2020-02-15)')"),
    rejectedWith("23P01"),
  );
  await assert.rejects(pool().execute("INSERT INTO room VALUES (2, NULL, 'west', '[2020-01-01,2020-02-01)')"), rejectedWith("23505"));
});

// [spec:pgorm:req:bindings.schema-tables/test]
test("a deferred key is checked at commit, an immediate one at each statement's end", async () => {
  await pool().execute(
    createTable("slot")
      .column(new ColumnDef("n", "integer"))
      .column(new ColumnDef("m", "integer"))
      .unique("n", { deferrability: "deferrableInitiallyDeferred" })
      .unique("m", { deferrability: "deferrableInitiallyImmediate" }),
  );
  await pool().execute("INSERT INTO slot VALUES (1, 1), (2, 2)");
  await pool().transaction(async (tx) => {
    await tx.execute("UPDATE slot SET n = 2 WHERE n = 1");
    await tx.execute("UPDATE slot SET n = 1 WHERE m = 2");
  });
  await pool().execute("UPDATE slot SET m = m + 1");
  await assert.rejects(
    pool().transaction(async (tx) => {
      await tx.execute("INSERT INTO slot VALUES (1, 9)");
    }),
    rejectedWith("23505"),
  );
});

// [spec:pgorm:req:bindings.schema-tables/test]
test("a foreign key's actions, deferral, enforcement and PERIOD mean what they say", async () => {
  await pool().execute(
    createTable("owner").column(new ColumnDef("id", "integer")).column(new ColumnDef("during", "daterange"))
      .primaryKey("id", { withoutOverlaps: "during" }).unique("id", { name: "owner_id" }),
  );
  await pool().execute(
    createTable("pet")
      .column(new ColumnDef("owner", "integer"))
      .column(new ColumnDef("loose", "integer"))
      .column(new ColumnDef("late", "integer"))
      .column(new ColumnDef("held", "daterange"))
      .foreignKey("owner", "owner", "id", { onDelete: "cascade", name: "pet_owner" })
      .foreignKey("loose", "owner", "id", { enforcement: "notEnforced", name: "pet_loose" })
      .foreignKey("late", "owner", "id", { deferrability: "deferrableInitiallyDeferred", name: "pet_late" })
      .foreignKey(["owner"], "owner", ["id"], { period: ["held", "during"], name: "pet_period" }),
  );
  await pool().execute("INSERT INTO owner VALUES (1, '[2020-01-01,2020-02-01)')");
  await pool().execute("INSERT INTO pet VALUES (1, 99, NULL, '[2020-01-05,2020-01-10)')");
  await assert.rejects(pool().execute("INSERT INTO pet VALUES (1, NULL, NULL, '[2020-01-20,2020-03-01)')"), rejectedWith("23503"));
  await pool().transaction(async (tx) => {
    await tx.execute("INSERT INTO pet VALUES (NULL, NULL, 2, NULL)");
    await tx.execute("INSERT INTO owner VALUES (2, '[2020-01-01,2020-02-01)')");
  });
  await pool().execute("DELETE FROM pet WHERE late = 2");
  await pool().execute("DELETE FROM owner WHERE id = 1");
  assert.equal((await pool().one("SELECT count(*) AS n FROM pet")).n, 0n);
  assert.deepStrictEqual(
    (await constraints("pet")).filter((row) => (row as string[])[1] === "f"),
    [
      ["pet_late", "f", true, true, null],
      ["pet_loose", "f", false, false, null],
      ["pet_owner", "f", true, true, null],
      ["pet_period", "f", true, true, null],
    ],
  );
});

// [spec:pgorm:req:bindings.schema-alter/test]
test("constraints added NOT VALID hold new rows at once and leave the old ones to validateConstraint", async () => {
  await pool().execute("CREATE TABLE ledger (id integer PRIMARY KEY, amount integer, ref integer)");
  await pool().execute("CREATE TABLE account (id integer PRIMARY KEY)");
  await pool().execute("INSERT INTO ledger VALUES (1, -5, 7), (2, NULL, NULL)");
  const ledger = new Table("ledger");
  await assert.rejects(pool().execute(alterTable(ledger).addCheck(col("amount").gte(0))), rejectedWith("23514"));
  await pool().execute(
    alterTable(ledger)
      .addCheck(col("amount").gte(0), { name: "amount_positive", notValid: true })
      .addNotNull("amount", { name: "amount_present", notValid: true })
      .addForeignKey("ref", "account", "id", { name: "ledger_account", notValid: true }),
  );
  assert.deepStrictEqual(await constraints("ledger"), [
    ["amount_positive", "c", false, true, false],
    ["amount_present", "n", false, true, false],
    ["ledger_account", "f", false, true, null],
    ["ledger_pkey", "p", true, true, null],
  ]);
  await assert.rejects(pool().execute("INSERT INTO ledger VALUES (3, -1, NULL)"), rejectedWith("23514"));
  await assert.rejects(pool().execute("INSERT INTO ledger VALUES (3, NULL, NULL)"), rejectedWith("23502"));
  await assert.rejects(pool().execute("INSERT INTO ledger VALUES (3, 1, 7)"), rejectedWith("23503"));
  await assert.rejects(pool().execute(alterTable(ledger).validateConstraint("amount_positive")), rejectedWith("23514"));
  await pool().execute("UPDATE ledger SET amount = 5, ref = NULL");
  await pool().execute(
    alterTable(ledger).validateConstraint("amount_positive").validateConstraint("amount_present")
      .validateConstraint("ledger_account"),
  );
  assert.ok((await constraints("ledger")).every((row) => (row as boolean[])[2]));
});

// [spec:pgorm:req:bindings.schema-alter/test]
test("ALTER TABLE adds, changes and drops columns, expressions and constraints", async () => {
  const stock = new Table("stock", { schema: "app" });
  await pool().execute(
    createTable(stock).column(new ColumnDef("id", "integer")).column(new ColumnDef("code", "smallint"))
      .column(new ColumnDef("twice", "integer").generated(col("id").mul(2), "stored"))
      .column(new ColumnDef("legacy", "text")),
  );
  await pool().execute("INSERT INTO app.stock (id, code) VALUES (1, 3)");
  await pool().execute(
    alterTable(stock)
      .addColumn(new ColumnDef("label", "text").default("none"))
      .addColumn(new ColumnDef("label", "text"), { ifNotExists: true })
      .modifyColumn(new ColumnDef("code", "bigint").notNull().default(1))
      .dropColumn("legacy")
      .addPrimaryKey("id", { name: "stock_key" })
      .setExpression("twice", col("id").mul(4)),
  );
  assert.deepStrictEqual(Object.values(await pool().one("SELECT * FROM app.stock")), [1, 3n, 4, "none"]);
  await pool().execute("INSERT INTO app.stock (id) VALUES (2)");
  await pool().execute(alterTable(stock).dropExpression("twice").modifyColumn(new ColumnDef("code").null()));
  await pool().execute("INSERT INTO app.stock (id, code, twice) VALUES (3, NULL, 5)");
  assert.deepStrictEqual(
    (await pool().query("SELECT code, twice FROM app.stock ORDER BY id")).map((row) => [row.code, row.twice]),
    [[3n, 4], [1n, 8], [null, 5]],
  );
  await pool().execute(alterTable(stock).dropConstraint("stock_key").dropConstraint("stock_key", { ifExists: true }));
  assert.deepStrictEqual(await constraints("app.stock"), [["stock_id_not_null", "n", true, true, false]]);
});

// [spec:pgorm:req:bindings.schema-alter/test]
test("alterConstraint moves a foreign key between enforced and not, and a NOT NULL between inherited and not", async () => {
  await pool().execute("CREATE TABLE tag (id integer PRIMARY KEY)");
  await pool().execute("CREATE TABLE label (tag integer CONSTRAINT label_tag REFERENCES tag, name text NOT NULL)");
  const label = new Table("label");
  await pool().execute(alterTable(label).alterConstraint("label_tag", "notEnforced"));
  await pool().execute("INSERT INTO label VALUES (1, 'orphan')");
  await assert.rejects(pool().execute(alterTable(label).alterConstraint("label_tag", "enforced")), rejectedWith("23503"));
  await pool().execute("DELETE FROM label");
  await pool().execute(alterTable(label).alterConstraint("label_tag", "enforced").alterConstraint("label_name_not_null", "noInherit"));
  assert.deepStrictEqual(await constraints("label"), [
    ["label_name_not_null", "n", true, true, true],
    ["label_tag", "f", true, true, null],
  ]);
});

// [spec:pgorm:req:bindings.schema-tables/test]
test("tables are renamed, emptied, commented and dropped, a CASCADE taking what depends on them", async () => {
  await pool().execute("CREATE TABLE app.draft (id integer CONSTRAINT draft_key PRIMARY KEY, body text)");
  await pool().execute("CREATE TABLE app.ref (draft integer REFERENCES app.draft)");
  await pool().execute("INSERT INTO app.draft VALUES (1, 'x')");
  const draft = new Table("draft", { schema: "app" });
  await pool().execute(renameColumn(draft, "body", "text"));
  await pool().execute(renameConstraint(draft, "draft_key", "draft_pk"));
  await pool().execute(renameTable(draft, "essay"));
  const essay = new Table("essay", { schema: "app" });
  await pool().execute(commentOnTable(essay, "it's \\ written"));
  await pool().execute(commentOnColumn(essay, "text", "line one\nline two"));
  const described = await pool().one(
    "SELECT obj_description('app.essay'::regclass, 'pg_class') AS t, col_description('app.essay'::regclass, 2) AS c",
  );
  assert.deepStrictEqual([described.t, described.c], ["it's \\ written", "line one\nline two"]);
  assert.deepStrictEqual(await constraints("app.essay"), [["draft_pk", "p", true, true, null]]);
  await assert.rejects(pool().execute(truncateTable(essay)), rejectedWith("0A000"));
  await pool().execute(truncateTable(new Table("ref", { schema: "app" })));
  await pool().execute(dropTable(new Table("ref", { schema: "app" })));
  await pool().execute(truncateTable(essay));
  assert.equal((await pool().one("SELECT count(*) AS n FROM app.essay")).n, 0n);
  await pool().execute("CREATE TABLE app.ref (essay integer REFERENCES app.essay)");
  await assert.rejects(pool().execute(dropTable(essay)), rejectedWith("2BP01"));
  await pool().execute(dropTable([essay, "missing"], { ifExists: true, behavior: "cascade" }));
  assert.deepStrictEqual(await constraints("app.ref"), []);
});

// [spec:pgorm:req:bindings.schema/test]
test("an ALTER TABLE with no action has nothing to inspect, and every terminal refuses it", async () => {
  const pending = alterTable("anything");
  assert.equal("inspect" in pending, false);
  for (const run of [() => pool().execute(pending as never), () => pool().query(pending as never)]) {
    await assert.rejects(run(), refused(/ALTER TABLE needs an action/));
  }
  const column = new ColumnDef("a", "integer");
  await assert.rejects(pool().execute(column as never), refused(/ColumnDef is not a statement to run/));
  assert.deepStrictEqual(createTable("t").inspect().values, []);
});

// [spec:pgorm:req:bindings.schema-api+3/test]
// [spec:pgorm:req:napi.typing+1/test]
test("every complete DDL statement is a DDL, a Statement the terminals take, made only by the module", () => {
  const statements: DDL[] = [
    createTable("t").column(new ColumnDef("a", "integer")),
    alterTable("t").dropColumn("a"),
    createIndex("t", "a"),
    createType("mood").values(["calm"]),
    createSequence("ticket"),
    createSchema("app"),
    renameSchema("app", "application"),
  ];
  const run: readonly Statement[] = statements;
  for (const statement of run) assert.ok(statement instanceof DDL);
  // @ts-expect-error a DDL comes from a DDL function, never its constructor
  assert.throws(() => new DDL(null), /a DDL statement is made by the module's DDL functions/);
  assert.equal("SchemaStatement" in pgorm, false);
  // @ts-expect-error "schema" names a database schema alone, so no DDL class is SchemaStatement
  assert.equal(pgorm.SchemaStatement, undefined);
});

// [spec:pgorm:req:bindings.schema-tables/test]
// [spec:pgorm:req:bindings.schema-alter/test]
// [spec:pgorm:req:bindings.argument-errors+1/test]
test("a shape PostgreSQL cannot take, or an option the builder does not know, is refused as it is built", () => {
  assert.throws(() => createTable("t").column(new ColumnDef("a")), refused(/needs a type/));
  assert.throws(() => alterTable("t").addColumn(new ColumnDef("a")), refused(/needs a type/));
  assert.throws(() => createTable(new Table("t", { alias: "x" })), refused(/without an alias/));
  assert.throws(() => createTable("t").primaryKey([] as never), refused(/at least one column/));
  assert.throws(() => createTable("t").foreignKey(["a", "b"], "u", ["x"] as never), refused(/as many referenced columns/));
  assert.throws(() => createTable("t").foreignKey("a", "u", "x", { period: ["p"] as never }), mistyped(/PERIOD pair/));
  assert.throws(
    () => alterTable("t").modifyColumn(new ColumnDef("a", "integer").generated(col("b"), "stored")),
    refused(/setExpression/),
  );
  assert.throws(() => alterTable("t").modifyColumn(new ColumnDef("a", "integer").autoIncrement()), refused(/identity/));
  assert.throws(() => alterTable("t").modifyColumn(new ColumnDef("a").collate("C")), refused(/collation only with its type/));
  assert.throws(() => alterTable("t").modifyColumn(new ColumnDef("a")), refused(/changes something/));
  assert.throws(() => new ColumnDef("a", "integer").generated(col("b"), "computed" as never), mistyped(/"stored", "virtual"/));
  assert.throws(() => new ColumnDef("a", "integer").identity("sometimes" as never), mistyped(/generation/));
  assert.throws(() => new ColumnDef("a", "integer").check(col("a").gt(0), { enforcement: "off" as never }), mistyped(/enforcement/));
  assert.throws(() => createTable("t").unique("a", { deferrability: "later" as never }), mistyped(/deferrability/));
  assert.throws(() => alterTable("t").alterConstraint("c", "valid" as never), mistyped(/change/));
  assert.throws(() => new ColumnDef("a", "integer").notNull({ nam: "x" } as never), TypeError);
  assert.throws(() => createTable("t").primaryKey("a", { nullsNotDistinct: true } as never), TypeError);
  assert.throws(() => alterTable("t").addCheck(col("a"), { notValid: "yes" } as never), TypeError);
  assert.throws(() => createTable("t").check(col("a").gt(0), "named" as never), TypeError);
  assert.throws(() => new ColumnDef("a", "integer").identity("always", { start: 1 } as never), TypeError);
  assert.throws(() => new ColumnDef("a", "integer").identity("always", { startWith: 1.5 }), refused(/startWith/));
  assert.throws(() => new ColumnDef("a", "integer").default(null as never), refused(/null has no kind/));
  assert.throws(() => new ColumnDef("", "integer"), refused(/1–63/));
  assert.throws(() => new ColumnDef("a", "integr" as never), refused(/no built-in type/));
});
