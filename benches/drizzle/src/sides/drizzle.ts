import { asc, eq, type SQL, sql } from "drizzle-orm";
import { drizzle as drizzlePg, type NodePgDatabase } from "drizzle-orm/node-postgres";
import { bigint, boolean, jsonb, numeric, PgDialect, pgTable, text, timestamp, uuid } from "drizzle-orm/pg-core";
import { drizzle as drizzlePostgres, type PostgresJsDatabase } from "drizzle-orm/postgres-js";
import type pg from "pg";
import type { Sql } from "postgres";

import {
  checkInt8,
  checkOneRow,
  checkRow,
  checkRows,
  checkWide,
  LOOKUP_ID,
  PARAMS16_SUM,
  PAYLOAD,
  SCALAR_VALUE,
  UPDATE_VALUES,
} from "../fixture.ts";
import type { Options } from "../options.ts";
import { PREPARED_SQL, preparedRows, type Session, type Side, type Workload } from "../sides.ts";
import { pgClient, pgPool } from "./pg.ts";
import { postgresClient } from "./postgresjs.ts";

const benchRow = pgTable("bench_row", {
  id: bigint("id", { mode: "bigint" }).primaryKey(),
  payload: text("payload").notNull(),
  value: bigint("value", { mode: "bigint" }).notNull(),
});

const benchWide = pgTable("bench_wide", {
  id: bigint("id", { mode: "bigint" }).primaryKey(),
  amount: numeric("amount", { precision: 20, scale: 6 }).notNull(),
  at: timestamp("at", { withTimezone: true }).notNull(),
  uid: uuid("uid").notNull(),
  label: text("label").notNull(),
  flag: boolean("flag").notNull(),
  doc: jsonb("doc").notNull(),
});

type Db = NodePgDatabase | PostgresJsDatabase;

/** What differs between Drizzle's two drivers: the shape `execute` resolves with. */
interface Driver {
  readonly name: "pg" | "postgres";
  /** The rows of a raw `execute`: a node-postgres result's `rows`, or postgres.js's row list itself. */
  rows(result: unknown): Record<string, unknown>[];
  /** The rows an update affected: `rowCount`, or the row list's `count`. */
  count(result: unknown): number | null;
}

const NODE_PG: Driver = {
  name: "pg",
  rows: (result) => (result as pg.QueryResult<Record<string, unknown>>).rows,
  count: (result) => (result as pg.QueryResult).rowCount,
};

const POSTGRES_JS: Driver = {
  name: "postgres",
  rows: (result) => result as Record<string, unknown>[],
  count: (result) => (result as { count: number }).count,
};

const v = SCALAR_VALUE;
const scalarSql = () => sql`SELECT ${v}::BIGINT`;
const params16Sql = () => sql`SELECT ${sql.join(Array.from({ length: 16 }, () => sql`${v}::BIGINT`), sql` + `)}`;
const UPDATE_NAMES = UPDATE_VALUES.map((_, at) => `u${at + 1}`);
const sumOf = (parts: readonly unknown[]) =>
  sql.join(parts.map((part) => sql`${part}::BIGINT`), sql` + `);

const lookupQuery = (db: Db) => db.select().from(benchRow).where(eq(benchRow.id, BigInt(LOOKUP_ID)));
const updateQuery = (db: Db) =>
  db.update(benchRow).set({ payload: PAYLOAD, value: sumOf(UPDATE_VALUES) }).where(eq(benchRow.id, BigInt(LOOKUP_ID)));
const rowsQuery = (db: Db, count: number) => db.select().from(benchRow).orderBy(asc(benchRow.id)).limit(count);
const wideQuery = (db: Db, count: number) => db.select().from(benchWide).orderBy(asc(benchWide.id)).limit(count);

/**
 * Each statement built once with `sql.placeholder`s and `.prepare(name)`ed,
 * which Drizzle hands its driver as a named statement.
 */
function preparedQueries(db: Db) {
  return {
    lookup: db.select().from(benchRow).where(eq(benchRow.id, sql.placeholder("id"))).prepare("drizzle_lookup"),
    update: db
      .update(benchRow)
      .set({ payload: sql`${sql.placeholder("payload")}`, value: sumOf(UPDATE_NAMES.map((name) => sql.placeholder(name))) })
      .where(eq(benchRow.id, sql.placeholder("id")))
      .prepare("drizzle_update16"),
    rows: db.select().from(benchRow).orderBy(asc(benchRow.id)).limit(sql.placeholder("count")).prepare("drizzle_rows"),
    wide: db.select().from(benchWide).orderBy(asc(benchWide.id)).limit(sql.placeholder("count")).prepare("drizzle_wide"),
  };
}

const updateValues: Record<string, unknown> = Object.fromEntries([
  ["id", BigInt(LOOKUP_ID)],
  ["payload", PAYLOAD],
  ...UPDATE_NAMES.map((name, at) => [name, UPDATE_VALUES[at]]),
]);

function session(db: Db, driver: Driver, prepared: boolean, readPrepared: () => Promise<Record<string, unknown>[]>, close: () => Promise<void>): Session {
  const raw = {
    scalar: {
      run: async () => driver.rows(await db.execute(scalarSql()))[0]?.int8,
      check: (value: unknown) => checkInt8(value, SCALAR_VALUE, "scalar"),
    },
    params16: {
      run: async () => driver.rows(await db.execute(params16Sql()))[0]?.["?column?"],
      check: (value: unknown) => checkInt8(value, PARAMS16_SUM, "params16"),
    },
    transaction: {
      run: () => db.transaction(async (tx) => driver.rows(await tx.execute(scalarSql()))[0]?.int8),
      check: (value: unknown) => checkInt8(value, SCALAR_VALUE, "transaction"),
    },
  };
  const statements = prepared ? preparedQueries(db) : undefined;
  return {
    fixed: statements
      ? {
          model_lookup: {
            run: async () => (await statements.lookup.execute({ id: BigInt(LOOKUP_ID) }))[0],
            check: (row) => checkRow(row, LOOKUP_ID, "model_lookup"),
          },
          update16: {
            run: async () => driver.count(await statements.update.execute(updateValues)),
            check: (count) => checkOneRow(count, "update16"),
          },
        }
      : {
          ...raw,
          model_lookup: { run: async () => (await lookupQuery(db))[0], check: (row) => checkRow(row, LOOKUP_ID, "model_lookup") },
          update16: { run: async () => driver.count(await updateQuery(db)), check: (count) => checkOneRow(count, "update16") },
        },
    counted: {
      rows_all: (count) => ({
        run: () => (statements ? statements.rows.execute({ count }) : rowsQuery(db, count)),
        check: (rows) => checkRows(rows, count, "rows_all"),
      }),
      wide_all: (count) => ({
        run: () => (statements ? statements.wide.execute({ count }) : wideQuery(db, count)),
        check: (rows) => checkWide(rows, count, "wide_all"),
      }),
    },
    prepared: async () => preparedRows(await readPrepared()),
    close,
  };
}

const NO_STREAM = "Drizzle has no row stream for PostgreSQL";
const NO_PREPARE = "Drizzle's raw db.execute(sql) has no .prepare(), and its select builder needs a FROM; the unprepared side runs it";

function rendered(prepared: boolean): Partial<Record<Workload, string>> {
  const dialect = new PgDialect();
  const db = drizzlePg.mock();
  const raw = (query: SQL) => dialect.sqlToQuery(query).sql;
  const statements = preparedQueries(db);
  const shown = (query: { getQuery(): { sql: string } }) => query.getQuery().sql;
  return prepared
    ? {
        model_lookup: shown(statements.lookup),
        update16: shown(statements.update),
        rows_all: shown(statements.rows),
        wide_all: shown(statements.wide),
      }
    : {
        scalar: raw(scalarSql()),
        params16: raw(params16Sql()),
        model_lookup: lookupQuery(db).toSQL().sql,
        update16: updateQuery(db).toSQL().sql,
        transaction: raw(scalarSql()),
        rows_all: rowsQuery(db, 128).toSQL().sql,
        wide_all: wideQuery(db, 128).toSQL().sql,
        concurrent_pool: raw(scalarSql()),
      };
}

function absent(prepared: boolean): Partial<Record<Workload, string>> {
  return prepared
    ? { scalar: NO_PREPARE, params16: NO_PREPARE, transaction: NO_PREPARE, concurrent_pool: NO_PREPARE, rows_stream: NO_STREAM }
    : { rows_stream: NO_STREAM };
}

export function drizzleSides(): Side[] {
  const sides: Side[] = [];
  for (const prepared of [true, false]) {
    const suffix = prepared ? "prepared" : "unprepared";
    sides.push({
      name: `drizzle-pg/${suffix}`,
      library: "drizzle",
      absent: absent(prepared),
      sql: rendered(prepared),
      async open(dsn: string) {
        const client = await pgClient(dsn, "pgorm-drizzle-bench");
        return session(
          drizzlePg({ client }),
          NODE_PG,
          prepared,
          async () => (await client.query(PREPARED_SQL)).rows,
          () => client.end(),
        );
      },
      ...(prepared
        ? {}
        : {
            async openPool(dsn: string, options: Options) {
              const pool = pgPool(dsn, options, "pgorm-drizzle-bench");
              const db = drizzlePg({ client: pool });
              return {
                operation: {
                  run: async () => NODE_PG.rows(await db.execute(scalarSql()))[0]?.int8,
                  check: (value: unknown) => checkInt8(value, SCALAR_VALUE, "concurrent_pool"),
                },
                close: () => pool.end(),
              };
            },
          }),
    });
    sides.push({
      name: `drizzle-postgres/${suffix}`,
      library: "drizzle",
      absent: absent(prepared),
      sql: rendered(prepared),
      async open(dsn: string) {
        // Drizzle replaces the client's date parsers, so it gets clients of its own.
        const client: Sql = postgresClient(dsn, 1, true);
        return session(
          drizzlePostgres({ client }),
          POSTGRES_JS,
          prepared,
          () => client.unsafe(PREPARED_SQL, [], { prepare: false }),
          () => client.end(),
        );
      },
      ...(prepared
        ? {}
        : {
            async openPool(dsn: string, options: Options) {
              const client = postgresClient(dsn, options.poolSize, true);
              const db = drizzlePostgres({ client });
              return {
                operation: {
                  run: async () => POSTGRES_JS.rows(await db.execute(scalarSql()))[0]?.int8,
                  check: (value: unknown) => checkInt8(value, SCALAR_VALUE, "concurrent_pool"),
                },
                close: () => client.end(),
              };
            },
          }),
    });
  }
  return sides;
}
