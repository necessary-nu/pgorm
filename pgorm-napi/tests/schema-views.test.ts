// Views, materialized views and CREATE TABLE AS built from JavaScript against a
// live server, in either runtime: what each statement makes, what reading and
// writing through it does, and what the server and the builders refuse.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import {
  bind,
  call,
  col,
  ColumnDef,
  ConstructionError,
  createIndex,
  createMaterializedView,
  createRecursiveView,
  createTable,
  createTableAs,
  createView,
  DatabaseError,
  dropMaterializedView,
  dropView,
  Pool,
  refreshMaterializedView,
  select,
  type Select,
  Table,
  Value,
} from "../lib/index.js";
import { mistyped, scratchDatabase } from "./support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

const account = new Table("account", { schema: "app" });
const app = (name: string) => new Table(name, { schema: "app" });

/** Every active account's `id`, `name` and `active`, the `TRUE` a literal in the definition. */
const activeAccounts = () =>
  select(col("id"), col("name"), col("active")).from(account).where(col("active").eq(true));

before(async () => {
  database = await scratchDatabase("pgorm_napi_schema_views");
  shared = new Pool(database.dsn, { maxSize: 4 });
  await pool().execute("CREATE SCHEMA app");
  await pool().execute(
    createTable(account)
      .column(new ColumnDef("id", "integer").identity("always"))
      .column(new ColumnDef("name", "text").notNull())
      .column(new ColumnDef("active", "boolean").notNull().default(true))
      .primaryKey("id"),
  );
  await pool().execute("INSERT INTO app.account (name, active) VALUES ('ada', true), ('it''s', true), ('cy', false)");
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

async function names(relation: string): Promise<string[]> {
  return (await pool().query(`SELECT name FROM ${relation} ORDER BY id`)).map((row) => String(row.name));
}

// [spec:pgorm:req:bindings.schema-views+3/test]
test("a view over a select reads the rows it shows, and a write through it reaches the table", async () => {
  const view = createView(app("active_account"), activeAccounts());
  assert.deepStrictEqual(view.inspect(), {
    sql: 'CREATE VIEW "app"."active_account" AS SELECT "id", "name", "active" FROM "app"."account" WHERE "active" = TRUE',
    values: [],
  });
  await pool().execute(view);
  assert.deepStrictEqual(await names("app.active_account"), ["ada", "it's"]);
  await pool().execute("INSERT INTO app.active_account (name) VALUES ('dee')");
  assert.deepStrictEqual(await names("app.account"), ["ada", "it's", "cy", "dee"]);

  const named = createView(app("named_account"), select(col("id")).from(account).where(col("name").eq("it's")))
    .columns(["account_id"]);
  await pool().execute(named);
  const definition = await pool().one("SELECT pg_get_viewdef('app.named_account'::regclass) AS d");
  assert.match(String(definition.d), /'it''s'::text/);
  const row = await pool().one("SELECT account_id FROM app.named_account");
  assert.equal(row.account_id, 2);

  await pool().execute(
    createView(app("active_account"), select(col("id"), col("name"), col("active"), col("id").as("again")).from(account))
      .orReplace()
      .securityBarrier(),
  );
  const options = await pool().one("SELECT reloptions::text AS o FROM pg_class WHERE oid = 'app.active_account'::regclass");
  assert.equal(options.o, "{security_barrier=true}");
  await assert.rejects(
    pool().execute(createView(app("active_account"), select(col("name")).from(account)).orReplace()),
    rejectedWith("42P16"),
  );
});

// [spec:pgorm:req:bindings.schema-views+3/test]
test("a check option refuses a row the view hides, and a view that cannot be updated takes none", async () => {
  await pool().execute(createView(app("checked_account"), activeAccounts()).checkOption("cascaded"));
  await pool().execute("INSERT INTO app.checked_account (name) VALUES ('ok')");
  await assert.rejects(
    pool().execute("INSERT INTO app.checked_account (name, active) VALUES ('hidden', false)"),
    rejectedWith("44000"),
  );
  await assert.rejects(
    pool().execute(createView(app("grouped"), select(col("active")).from(account).groupBy(col("active"))).checkOption("local")),
    rejectedWith("0A000"),
  );
});

// [spec:pgorm:req:bindings.schema-views+3/test]
test("a recursive view reads itself by the columns it names", async () => {
  const counting = select(bind(new Value(1, "i32")).as("n"))
    .unionAll(select(col("n").add(1)).from(new Table("counter")).where(col("n").lt(4)));
  const view = createRecursiveView("counter", "n", counting);
  assert.match(view.inspect().sql, /^CREATE RECURSIVE VIEW "counter" \("n"\) AS SELECT 1 AS "n" UNION ALL/);
  await pool().execute(view);
  const rows = await pool().query("SELECT n FROM counter ORDER BY n");
  assert.deepStrictEqual(rows.map((row) => row.n), [1, 2, 3, 4]);
});

// [spec:pgorm:req:bindings.schema-views+3/test]
test("a materialized view holds its rows until refreshed, concurrently once it has a unique index", async () => {
  const total = app("account_total");
  await pool().execute(createMaterializedView(total, activeAccounts()).columns(["account_id"]).withNoData());
  await assert.rejects(pool().query("SELECT * FROM app.account_total"), rejectedWith("55000"));
  await assert.rejects(pool().execute(refreshMaterializedView(total, { concurrently: true })), rejectedWith("0A000"));
  await pool().execute(refreshMaterializedView(total));
  const count = async () => Number((await pool().one("SELECT count(*)::int AS c FROM app.account_total")).c);
  const before = await count();
  await pool().execute("INSERT INTO app.account (name) VALUES ('later')");
  assert.equal(await count(), before);
  await assert.rejects(pool().execute(refreshMaterializedView(total, { concurrently: true })), rejectedWith("55000"));
  await pool().execute(createIndex(total, "account_id", { name: "account_total_id" }).unique());
  await pool().execute(refreshMaterializedView(total, { concurrently: true }));
  assert.equal(await count(), before + 1);
  await pool().execute(refreshMaterializedView(total, { withNoData: true }));
  await assert.rejects(pool().query("SELECT * FROM app.account_total"), rejectedWith("55000"));
  await pool().execute(createMaterializedView(total, select(col("id")).from(account)).ifNotExists());

  await assert.rejects(pool().execute(dropView(total)), rejectedWith("42809"));
  await pool().execute(dropMaterializedView([total, app("missing")], { ifExists: true, behavior: "cascade" }));
  const left = await pool().one("SELECT count(*)::int AS c FROM pg_class WHERE relname = 'account_total'");
  assert.equal(left.c, 0);
});

// [spec:pgorm:req:bindings.schema-views+3/test]
test("CREATE TABLE AS WITH NO DATA makes the query's columns, and the table then takes rows", async () => {
  const copy = app("account_copy");
  await pool().execute(createTableAs(copy, activeAccounts()).columns(["account_id"]).withNoData());
  assert.equal((await pool().one("SELECT count(*)::int AS c FROM app.account_copy")).c, 0);
  const columns = await pool().query(
    "SELECT attname::text AS a FROM pg_attribute WHERE attrelid = 'app.account_copy'::regclass AND attnum > 0 ORDER BY attnum",
  );
  assert.deepStrictEqual(columns.map((row) => row.a), ["account_id", "name", "active"]);
  await pool().execute("INSERT INTO app.account_copy SELECT id, name, active FROM app.account WHERE id < 3");
  const rows = await pool().query("SELECT name FROM app.account_copy ORDER BY account_id");
  assert.deepStrictEqual(rows.map((row) => row.name), ["ada", "it's"]);
  await pool().execute(createTableAs(copy, select(col("id")).from(account)).ifNotExists());
  const kind = await pool().one("SELECT relkind::text AS k FROM pg_class WHERE oid = 'app.account_copy'::regclass");
  assert.equal(kind.k, "r");
  await assert.rejects(pool().execute(dropView(copy)), rejectedWith("42809"));

  await pool().execute(createView(app("over_copy"), select(col("name")).from(copy)));
  await pool().execute(createView(app("over_over"), select(col("name")).from(app("over_copy"))));
  await assert.rejects(pool().execute(dropView(app("over_copy"), { behavior: "restrict" })), rejectedWith("2BP01"));
  await pool().execute(dropView([app("over_copy"), app("missing")], { ifExists: true, behavior: "cascade" }));
  const left = await pool().one("SELECT count(*)::int AS c FROM pg_class WHERE relname IN ('over_copy', 'over_over')");
  assert.equal(left.c, 0);
});

// [spec:pgorm:req:bindings.schema-views+3/test]
test("view statements PostgreSQL cannot take are refused as they are built", () => {
  assert.throws(() => createRecursiveView("counter", [] as never, select(col("n"))), refused(/at least one column/));
  assert.throws(
    () => refreshMaterializedView("m", { concurrently: true, withNoData: true }),
    refused(/never both/),
  );
  assert.throws(() => refreshMaterializedView("m", { cascade: true } as never), TypeError);
  assert.throws(() => dropView("v", { force: true } as never), TypeError);
  assert.throws(() => createView("v", new Table("t") as never), mistyped(/expected a Select/));
  assert.throws(() => createView("v", select(col("a"))).checkOption("both" as never), mistyped(/"local", "cascaded"/));
  assert.throws(() => dropMaterializedView([] as never), refused(/at least one relation/));
  const recursive = createRecursiveView("counter", ["n"], select(col("n")));
  assert.equal("checkOption" in recursive, false);
  assert.equal("ifNotExists" in createView("v", select(col("a"))), false);
  assert.equal("orReplace" in createMaterializedView("m", select(col("a"))), false);
  assert.deepStrictEqual(
    createRecursiveView("counter", ["n"], select(col("n"))).columns(["m"]).securityInvoker().inspect().sql,
    'CREATE RECURSIVE VIEW "counter" ("n", "m") WITH (security_invoker) AS SELECT "n"',
  );
});

// [spec:pgorm:req:bindings.schema-views+3/test]
test("a body whose tie limit or window PostgreSQL refuses is refused as each builder is called", () => {
  const step = select(col("n").add(1)).from(new Table("r")).where(col("n").lt(4));
  const builders: [string, (body: Select) => { inspect(): { sql: string } }][] = [
    ["createView", (body) => createView("v", body)],
    ["createRecursiveView", (body) => createRecursiveView("r", ["n"], body.unionAll(step))],
    ["createMaterializedView", (body) => createMaterializedView("m", body)],
    ["createTableAs", (body) => createTableAs("t", body)],
  ];
  const unordered = select(col("id").as("n")).from(account).limitWithTies(2);
  const undeclared = select(call("count", col("id")).over("w").as("n")).from(account);
  for (const [name, build] of builders) {
    assert.throws(() => build(unordered), refused(/needs an ORDER BY/), name);
    assert.throws(() => build(undeclared), refused(/no window "w" is declared/), name);
    assert.match(build(unordered.orderBy(col("id").asc())).inspect().sql, /ORDER BY "id" ASC FETCH FIRST 2 ROWS WITH TIES/, name);
  }
});
