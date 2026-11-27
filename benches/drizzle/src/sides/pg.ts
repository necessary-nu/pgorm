import pg from "pg";
import QueryStream from "pg-query-stream";

import {
  checkInt8,
  checkOneRow,
  checkRow,
  checkRows,
  checkTexts,
  checkWide,
  checkWideColumn,
  LOOKUP,
  LOOKUP_ID,
  PARAMS16,
  PARAMS16_SUM,
  PAYLOAD,
  ROWS,
  SCALAR,
  SCALAR_VALUE,
  TEXTS,
  UPDATE16,
  UPDATE_VALUES,
  WIDE,
} from "../fixture.ts";
import type { Options } from "../options.ts";
import { PREPARED_SQL, preparedRows, type Session, type Side } from "../sides.ts";

/**
 * A query config: named, which node-postgres parses once per connection
 * and reuses, or unnamed, parsed again on every call.
 */
function config(name: string | undefined, text: string, values: unknown[]): pg.QueryConfig {
  return name === undefined ? { text, values } : { name, text, values };
}

async function streamed(client: pg.Client, text: string, values: unknown[], batchSize: number): Promise<unknown[]> {
  const rows: unknown[] = [];
  for await (const row of client.query(new QueryStream(text, values, { batchSize }))) rows.push(row);
  return rows;
}

function session(client: pg.Client, named: boolean, options: Options): Session {
  const name = (statement: string) => (named ? statement : undefined);
  const scalar = () => client.query(config(name("scalar"), SCALAR, [SCALAR_VALUE]));
  return {
    fixed: {
      scalar: { run: async () => (await scalar()).rows[0].int8, check: (value) => checkInt8(value, SCALAR_VALUE, "scalar") },
      params16: {
        run: async () => (await client.query(config(name("params16"), PARAMS16, Array.from({ length: 16 }, () => SCALAR_VALUE)))).rows[0]["?column?"],
        check: (value) => checkInt8(value, PARAMS16_SUM, "params16"),
      },
      model_lookup: {
        run: async () => (await client.query(config(name("lookup"), LOOKUP, [LOOKUP_ID]))).rows[0],
        check: (row) => checkRow(row, LOOKUP_ID, "model_lookup"),
      },
      update16: {
        run: async () => (await client.query(config(name("update16"), UPDATE16, [LOOKUP_ID, PAYLOAD, ...UPDATE_VALUES]))).rowCount,
        check: (count) => checkOneRow(count, "update16"),
      },
      transaction: {
        run: async () => {
          await client.query("BEGIN");
          try {
            const value = (await scalar()).rows[0].int8;
            await client.query("COMMIT");
            return value;
          } catch (error) {
            await client.query("ROLLBACK");
            throw error;
          }
        },
        check: (value) => checkInt8(value, SCALAR_VALUE, "transaction"),
      },
    },
    counted: {
      rows_all: (count) => ({
        run: async () => (await client.query(config(name("rows"), ROWS, [count]))).rows,
        check: (rows) => checkRows(rows, count, "rows_all"),
      }),
      rows_stream: (count) => ({
        run: () => streamed(client, ROWS, [count], options.cursorBatch),
        check: (rows) => checkRows(rows, count, "rows_stream"),
      }),
      wide_all: (count) => ({
        run: async () => (await client.query(config(name("wide"), WIDE, [count]))).rows,
        check: (rows) => checkWide(rows, count, "wide_all"),
      }),
      column: (column, count) => ({
        run: async () => (await client.query(config(name(`column_${column}`), `SELECT ${column} FROM bench_wide ORDER BY id LIMIT $1`, [count]))).rows,
        check: (rows) => checkWideColumn(rows, column, count, `column ${column}`),
      }),
      texts: (width) => ({
        run: async () => (await client.query(config(name("texts"), TEXTS, [width]))).rows,
        check: (rows) => checkTexts(rows, width, `texts ${width}`),
      }),
    },
    async prepared() {
      // Text with no values goes through the simple-query protocol, which prepares nothing.
      return preparedRows((await client.query(PREPARED_SQL)).rows);
    },
    close: () => client.end(),
  };
}

export async function pgClient(dsn: string, applicationName: string): Promise<pg.Client> {
  const client = new pg.Client({ connectionString: dsn, application_name: applicationName });
  await client.connect();
  return client;
}

export function pgPool(dsn: string, options: Options, applicationName: string): pg.Pool {
  return new pg.Pool({ connectionString: dsn, max: options.poolSize, application_name: applicationName });
}

const STREAM_UNNAMED = "pg-query-stream (over pg-cursor) always parses an unnamed statement";

export function pgSides(): Side[] {
  return [true, false].map((named) => ({
    name: named ? "pg/prepared" : "pg/unprepared",
    library: "driver",
    absent: {},
    sql: { scalar: SCALAR, params16: PARAMS16, model_lookup: LOOKUP, update16: UPDATE16, transaction: SCALAR, rows_all: ROWS, rows_stream: named ? `${ROWS} -- ${STREAM_UNNAMED}` : ROWS, wide_all: WIDE, concurrent_pool: SCALAR },
    async open(dsn: string, options: Options) {
      return session(await pgClient(dsn, "pgorm-drizzle-bench"), named, options);
    },
    async openPool(dsn: string, options: Options) {
      const pool = pgPool(dsn, options, "pgorm-drizzle-bench");
      return {
        operation: {
          run: async () => (await pool.query(config(named ? "scalar" : undefined, SCALAR, [SCALAR_VALUE]))).rows[0].int8,
          check: (value: unknown) => checkInt8(value, SCALAR_VALUE, "concurrent_pool"),
        },
        close: () => pool.end(),
      };
    },
  }));
}
