import postgres, { type Sql } from "postgres";

import {
  checkInt8,
  checkOneRow,
  checkRow,
  checkRows,
  checkTexts,
  checkWide,
  checkWideColumn,
  LOOKUP_ID,
  PARAMS16_SUM,
  PAYLOAD,
  SCALAR_VALUE,
  UPDATE_VALUES,
} from "../fixture.ts";
import type { Options } from "../options.ts";
import { PREPARED_SQL, preparedRows, type Session, type Side } from "../sides.ts";

export function postgresClient(dsn: string, max: number, prepare: boolean): Sql {
  return postgres(dsn, { max, prepare, onnotice: () => {}, connection: { application_name: "pgorm-drizzle-bench" } });
}

const v = SCALAR_VALUE;
const [u1, u2, u3, u4, u5, u6, u7, u8, u9, u10, u11, u12, u13, u14] = UPDATE_VALUES as [number, number, number, number, number, number, number, number, number, number, number, number, number, number];

async function streamed(query: AsyncIterable<readonly unknown[]>): Promise<unknown[]> {
  const rows: unknown[] = [];
  for await (const batch of query) for (const row of batch) rows.push(row);
  return rows;
}

function session(sql: Sql, options: Options): Session {
  return {
    fixed: {
      scalar: { run: async () => (await sql`SELECT ${v}::BIGINT`)[0]?.int8, check: (value) => checkInt8(value, SCALAR_VALUE, "scalar") },
      params16: {
        run: async () =>
          (
            await sql`SELECT ${v}::BIGINT + ${v}::BIGINT + ${v}::BIGINT + ${v}::BIGINT + ${v}::BIGINT + ${v}::BIGINT + ${v}::BIGINT + ${v}::BIGINT + ${v}::BIGINT + ${v}::BIGINT + ${v}::BIGINT + ${v}::BIGINT + ${v}::BIGINT + ${v}::BIGINT + ${v}::BIGINT + ${v}::BIGINT`
          )[0]?.["?column?"],
        check: (value) => checkInt8(value, PARAMS16_SUM, "params16"),
      },
      model_lookup: {
        run: async () => (await sql`SELECT id, payload, value FROM bench_row WHERE id = ${LOOKUP_ID}`)[0],
        check: (row) => checkRow(row, LOOKUP_ID, "model_lookup"),
      },
      update16: {
        run: async () =>
          (
            await sql`UPDATE bench_row SET payload = ${PAYLOAD}, value = ${u1}::BIGINT + ${u2}::BIGINT + ${u3}::BIGINT + ${u4}::BIGINT + ${u5}::BIGINT + ${u6}::BIGINT + ${u7}::BIGINT + ${u8}::BIGINT + ${u9}::BIGINT + ${u10}::BIGINT + ${u11}::BIGINT + ${u12}::BIGINT + ${u13}::BIGINT + ${u14}::BIGINT WHERE id = ${LOOKUP_ID}`
          ).count,
        check: (count) => checkOneRow(count, "update16"),
      },
      transaction: {
        run: () => sql.begin(async (tx) => (await tx`SELECT ${v}::BIGINT`)[0]?.int8),
        check: (value) => checkInt8(value, SCALAR_VALUE, "transaction"),
      },
    },
    counted: {
      rows_all: (count) => ({
        run: () => sql`SELECT id, payload, value FROM bench_row ORDER BY id LIMIT ${count}`,
        check: (rows) => checkRows(rows, count, "rows_all"),
      }),
      rows_stream: (count) => ({
        run: () => streamed(sql`SELECT id, payload, value FROM bench_row ORDER BY id LIMIT ${count}`.cursor(options.cursorBatch)),
        check: (rows) => checkRows(rows, count, "rows_stream"),
      }),
      wide_all: (count) => ({
        run: () => sql`SELECT id, amount, at, uid, label, flag, doc FROM bench_wide ORDER BY id LIMIT ${count}`,
        check: (rows) => checkWide(rows, count, "wide_all"),
      }),
      column: (column, count) => ({
        run: () => sql`SELECT ${sql(column)} FROM bench_wide ORDER BY id LIMIT ${count}`,
        check: (rows) => checkWideColumn(rows, column, count, `column ${column}`),
      }),
      texts: (width) => ({
        run: () => sql`SELECT id, body FROM bench_text WHERE width = ${width} ORDER BY id`,
        check: (rows) => checkTexts(rows, width, `texts ${width}`),
      }),
    },
    async prepared() {
      return preparedRows(await sql.unsafe(PREPARED_SQL, [], { prepare: false }));
    },
    close: () => sql.end(),
  };
}

const PARAMS16_TEXT = `SELECT ${Array.from({ length: 16 }, (_, at) => `$${at + 1}::BIGINT`).join(" + ")}`;

export function postgresSides(): Side[] {
  return [true, false].map((prepare) => ({
    name: prepare ? "postgres/prepared" : "postgres/unprepared",
    library: "driver",
    absent: {},
    sql: {
      scalar: "SELECT $1::BIGINT",
      params16: PARAMS16_TEXT,
      model_lookup: "SELECT id, payload, value FROM bench_row WHERE id = $1",
      update16: `UPDATE bench_row SET payload = $1, value = ${Array.from({ length: 14 }, (_, at) => `$${at + 2}::BIGINT`).join(" + ")} WHERE id = $16`,
      transaction: "SELECT $1::BIGINT",
      rows_all: "SELECT id, payload, value FROM bench_row ORDER BY id LIMIT $1",
      rows_stream: "SELECT id, payload, value FROM bench_row ORDER BY id LIMIT $1",
      wide_all: "SELECT id, amount, at, uid, label, flag, doc FROM bench_wide ORDER BY id LIMIT $1",
      concurrent_pool: "SELECT $1::BIGINT",
    },
    async open(dsn: string, options: Options) {
      return session(postgresClient(dsn, 1, prepare), options);
    },
    async openPool(dsn: string, options: Options) {
      const sql = postgresClient(dsn, options.poolSize, prepare);
      return {
        operation: {
          run: async () => (await sql`SELECT ${v}::BIGINT`)[0]?.int8,
          check: (value: unknown) => checkInt8(value, SCALAR_VALUE, "concurrent_pool"),
        },
        close: () => sql.end(),
      };
    },
  }));
}
