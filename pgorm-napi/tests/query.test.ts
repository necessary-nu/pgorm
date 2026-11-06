// A bound query through pgorm's pool settles a promise, in either runtime.
//
// The suite uses the public API through its declarations, which `deno test`
// and `deno check` type-check before anything runs.
// [spec:pgorm:req:napi.typing+1/test]

import assert from "node:assert/strict";
import { after, test } from "node:test";

import {
  CheckViolationError,
  ConnectionError,
  ConstructionError,
  DatabaseError,
  Decimal,
  DecodeError,
  ExclusionViolationError,
  ForeignKeyViolationError,
  NotNullViolationError,
  PgormError,
  Pool,
  UniqueViolationError,
  version,
} from "../lib/index.js";
import { dsn, scratchDatabase } from "./support.ts";

const pool = new Pool(dsn(), { maxSize: 16 });

after(async () => {
  await pool.close();
});

async function scalar(sql: string, params: readonly (number | bigint | string)[] = []): Promise<unknown> {
  return (await pool.one(sql, params)).n;
}

// [spec:pgorm:req:napi.loading+1/test]
// [spec:pgorm:def:bindings.api/test]
// [spec:pgorm:def:napi.api+1/test]
test("the addon loads and reports the pgorm release it was built from", () => {
  assert.match(version, /^\d+\.\d+\.\d+/);
});

// [spec:pgorm:req:napi.promises/test]
// [spec:pgorm:req:napi.raw/test]
// [spec:pgorm:req:bindings.results+2/test]
test("a bound query resolves with its result", async () => {
  assert.equal(await scalar("SELECT $1::int + 1 AS n", [41]), 42);
});

// [spec:pgorm:req:napi.promises/test]
test("a hundred queries in flight at once each resolve with their own result", async () => {
  const values = Array.from({ length: 100 }, (_, index) => index * 1000 - 50_000);
  const pending = values.map((value) => scalar("SELECT $1::int + 1 AS n", [value]));
  assert.deepEqual(await Promise.all(pending), values.map((value) => value + 1));
});

// [spec:pgorm:req:napi.promises/test]
test("queries that sleep on the server overlap rather than queue behind one another", async () => {
  const started = performance.now();
  const results = await Promise.all(
    Array.from({ length: 8 }, (_, index) => scalar("SELECT $1::int + 1 AS n FROM pg_sleep(0.5)", [index])),
  );
  assert.deepEqual(results, [1, 2, 3, 4, 5, 6, 7, 8]);
  assert.ok(performance.now() - started < 3000, "eight half-second queries ran concurrently");
});

// [spec:pgorm:req:bindings.errors+6/test]
test("an error PostgreSQL reports rejects with a DatabaseError carrying its SQLSTATE", async () => {
  const error = await scalar("SELECT $1::int + 1 AS n", [2147483647]).then(
    () => assert.fail("the overflow resolved"),
    (error: unknown) => error,
  );
  assert.ok(error instanceof DatabaseError);
  assert.ok(error instanceof PgormError);
  assert.ok(error instanceof Error);
  assert.equal(error.name, "DatabaseError");
  assert.equal(error.sqlstate, "22003");
  assert.equal(error.severity, "ERROR");
  assert.equal(error.message, "integer out of range");
  assert.equal(error.detail, null);
});

// [spec:pgorm:sem:error.model.parameter-count/test]    a statement given another number of values than it takes is the caller's ConstructionError, in pgorm's words
test("a statement given another number of values than its placeholders take rejects with a ConstructionError naming both", async () => {
  await assert.rejects(pool.one("SELECT $1 AS x, $2 AS y", [1]), (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `${error}`);
    assert.equal(error.message, "the statement takes 2 parameters, and 1 was given");
    return true;
  });
  await assert.rejects(pool.execute("SELECT 1", [1, 2]), (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `${error}`);
    assert.equal(error.message, "the statement takes 0 parameters, and 2 were given");
    return true;
  });
  assert.equal(await scalar("SELECT 1 AS n"), 1);
});

// [spec:pgorm:req:conn.copy.statement/test]    a COPY given as a statement is the caller's ConstructionError, nothing sent
test("a COPY given as a statement rejects with a ConstructionError naming copyIn and copyOut", async () => {
  for (
    const run of [
      () => pool.query("COPY (SELECT 1) TO STDOUT"),
      () => pool.execute("/* rows */ copy pg_class FROM STDIN"),
      () => pool.batchExecute("SELECT 1; COPY pg_class FROM STDIN"),
    ]
  ) {
    await assert.rejects(run(), (error: unknown) => {
      assert.ok(error instanceof ConstructionError, `${error}`);
      assert.match(error.message, /copyIn or copyOut/);
      return true;
    });
  }
  assert.equal((await pool.one("SELECT 'COPY' AS copy")).copy, "COPY");
});

// [spec:pgorm:req:bindings.errors+6/test]
test("a constraint violation rejects with the DatabaseError subclass named for it, carrying the names the server reported", async () => {
  const database = await scratchDatabase("pgorm_napi_violations");
  try {
    await using scratch = new Pool(database.dsn, { maxSize: 1 });
    for (
      const sql of [
        "CREATE DOMAIN positive AS int4 NOT NULL CONSTRAINT positive_check CHECK (VALUE > 0)",
        "CREATE TABLE parent (id int4 PRIMARY KEY)",
        `CREATE TABLE gauge (
          id int4 PRIMARY KEY,
          reading int4 NOT NULL CONSTRAINT reading_bounded CHECK (reading < 100),
          level positive,
          parent_id int4 CONSTRAINT gauge_parent REFERENCES parent,
          span int4range,
          CONSTRAINT gauge_apart EXCLUDE USING gist (span WITH &&)
        )`,
        "INSERT INTO gauge (id, reading, level, span) VALUES (1, 1, 1, '[1,5)')",
      ]
    ) {
      await scratch.execute(sql);
    }
    const insert = "INSERT INTO gauge (id, reading, level, parent_id, span) VALUES ";
    const classes = [
      UniqueViolationError,
      ForeignKeyViolationError,
      CheckViolationError,
      NotNullViolationError,
      ExclusionViolationError,
    ];
    const cases: [string, (typeof classes)[number], string, Record<string, string>][] = [
      [`${insert}(1, 1, 1, NULL, NULL)`, UniqueViolationError, "23505", { table: "gauge", constraint: "gauge_pkey" }],
      [`${insert}(2, 1, 1, 9, NULL)`, ForeignKeyViolationError, "23503", { table: "gauge", constraint: "gauge_parent" }],
      [`${insert}(2, 100, 1, NULL, NULL)`, CheckViolationError, "23514", {
        table: "gauge",
        constraint: "reading_bounded",
      }],
      [`${insert}(2, 1, 0, NULL, NULL)`, CheckViolationError, "23514", {
        datatype: "positive",
        constraint: "positive_check",
      }],
      [`${insert}(2, NULL, 1, NULL, NULL)`, NotNullViolationError, "23502", { table: "gauge", column: "reading" }],
      ["INSERT INTO gauge (id, reading) VALUES (2, 1)", NotNullViolationError, "23502", { datatype: "positive" }],
      [`${insert}(2, 1, 1, NULL, '[4,8)')`, ExclusionViolationError, "23P01", { table: "gauge", constraint: "gauge_apart" }],
    ];
    for (const [sql, expected, sqlstate, names] of cases) {
      await assert.rejects(scratch.execute(sql), (error: unknown) => {
        assert.ok(error instanceof expected, `${sqlstate}: ${error}`);
        assert.ok(error instanceof DatabaseError);
        assert.ok(error instanceof PgormError);
        for (const other of classes) {
          if (other !== expected) assert.ok(!(error instanceof other), `${sqlstate} is not a ${other.name}`);
        }
        assert.equal(error.name, expected.name);
        assert.equal(error.sqlstate, sqlstate);
        assert.equal(error.severity, "ERROR");
        assert.deepStrictEqual(
          {
            schema: error.schema,
            table: error.table,
            column: error.column,
            datatype: error.datatype,
            constraint: error.constraint,
          },
          { schema: "public", table: null, column: null, datatype: null, constraint: null, ...names },
        );
        return true;
      });
    }
    const other = await scratch.execute("SELECT 1 / 0").then(
      () => assert.fail("the division resolved"),
      (error: unknown) => error,
    );
    assert.ok(other instanceof DatabaseError);
    assert.equal(Object.getPrototypeOf(other), DatabaseError.prototype);
    assert.equal(other.sqlstate, "22012");
    assert.equal(other.datatype, null);
  } finally {
    await database.drop();
  }
});

// [spec:pgorm:req:bindings.errors+6/test]
test("a statement PostgreSQL cannot parse rejects with its syntax-error SQLSTATE", async () => {
  await assert.rejects(pool.query("SELEC 1"), (error: unknown) => {
    assert.ok(error instanceof DatabaseError);
    assert.equal(error.sqlstate, "42601");
    return true;
  });
});

// [spec:pgorm:req:bindings.errors+6/test]
test("an unreachable server rejects with a ConnectionError", async () => {
  await using unreachable = new Pool("postgres://pgorm@127.0.0.1:1/postgres?sslmode=disable");
  await assert.rejects(unreachable.query("SELECT 1"), (error: unknown) => {
    assert.ok(error instanceof ConnectionError);
    assert.ok(!("sqlstate" in error));
    return true;
  });
});

// [spec:pgorm:req:bindings.errors+6/test]
test("a password never appears in the error its connection failure rejects with", async () => {
  await using unreachable = new Pool("postgres://pgorm:hunter2-secret@127.0.0.1:1/postgres?sslmode=disable");
  await assert.rejects(
    unreachable.query("SELECT 1"),
    (error: unknown) => {
      assert.ok(error instanceof ConnectionError);
      assert.ok(!String(error.message).includes("hunter2-secret"));
      return true;
    },
  );
});

// [spec:pgorm:req:bindings.errors+6/test]
test("a parameter with no exact int4 rejects before anything is sent", async () => {
  for (const value of [1.5, 2 ** 31, -(2 ** 31) - 1, Number.NaN, Number.POSITIVE_INFINITY, "1"]) {
    await assert.rejects(scalar("SELECT $1::int + 1 AS n", [value]), (error: unknown) => {
      assert.ok(error instanceof ConstructionError, `${value}: ${error}`);
      assert.match(error.message, /parameter \$1/);
      return true;
    });
  }
});

// [spec:pgorm:req:bindings.errors+6/test]    a value its parameter's type refuses is a
// ConstructionError naming the parameter and carrying pgorm's refusal, as Python reports it
test("a bind refusal names the parameter and pgorm's reason", async () => {
  for (
    const [sql, value, refusal] of [
      ["SELECT $1::int + 1 AS n", "1", "cannot bind a `String` value to Postgres type `int4`"],
      ["SELECT $1::money AS m", new Decimal("1.5"), "cannot bind a `Decimal` value to Postgres type `money`"],
    ] as const
  ) {
    await assert.rejects(pool.query(sql, [value]), (error: unknown) => {
      assert.ok(error instanceof ConstructionError, `${sql}: ${error}`);
      assert.equal(error.message, `parameter $1 cannot be bound: ${refusal}`);
      return true;
    });
  }
});

// [spec:pgorm:req:bindings.errors+6/test]
test("an unusable connection string is a ConstructionError", () => {
  assert.throws(() => new Pool("postgres://["), ConstructionError);
});

// [spec:pgorm:req:bindings.errors+6/test]
test("a result the binding cannot decode exactly rejects with a DecodeError", async () => {
  await assert.rejects(pool.query("SELECT 'NaN'::numeric AS n"), DecodeError);
  await assert.rejects(pool.query("SELECT point(1, 2) AS n"), DecodeError);
});

// [spec:pgorm:req:bindings.argument-errors+1/test]
test("an argument of the wrong JavaScript type rejects with a TypeError", async () => {
  await assert.rejects(pool.query(42 as never), TypeError);
  await assert.rejects(pool.query("SELECT 1", [], { tagged: "yes" as never }), TypeError);
  await assert.rejects(pool.query("SELECT 1", 1 as never), TypeError);
});
