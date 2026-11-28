import { join } from "node:path";
import { pathToFileURL } from "node:url";

import { column, type Connection, model, Pool, template } from "@necessary/pgorm";

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
import { op, type Operation } from "../measure.ts";
import { checkout, type Options } from "../options.ts";
import { PREPARED_SQL, preparedRows, type Session, type Side } from "../sides.ts";

const BenchRow = model("bench_row", {
  columns: {
    id: column("i64", { primaryKey: true }),
    payload: column("text"),
    value: column("i64"),
  },
});

const BenchWide = model("bench_wide", {
  columns: {
    id: column("i64", { primaryKey: true }),
    amount: column("decimal"),
    at: column("datetime_utc"),
    uid: column("uuid"),
    label: column("text"),
    flag: column("bool"),
    doc: column("json"),
  },
});

const [first, ...rest] = UPDATE_VALUES as [number, ...number[]];

/** The update's sum of fourteen bound values, each cast as the raw statement casts it. */
function sum() {
  const [a, b, c, d, e, f, g, h, i, j, k, l, m] = rest as [number, number, number, number, number, number, number, number, number, number, number, number, number];
  return template`${first}::BIGINT + ${a}::BIGINT + ${b}::BIGINT + ${c}::BIGINT + ${d}::BIGINT + ${e}::BIGINT + ${f}::BIGINT + ${g}::BIGINT + ${h}::BIGINT + ${i}::BIGINT + ${j}::BIGINT + ${k}::BIGINT + ${l}::BIGINT + ${m}::BIGINT`;
}

const lookup = () => BenchRow.findByKey({ id: BigInt(LOOKUP_ID) });
const update = () => BenchRow.update({ payload: PAYLOAD, value: sum() }).where(BenchRow.key({ id: BigInt(LOOKUP_ID) }));
const rowsQuery = (count: number) => BenchRow.find().orderBy(BenchRow.col("id").asc()).limit(count);
const wideQuery = (count: number) => BenchWide.find().orderBy(BenchWide.col("id").asc()).limit(count);

/**
 * The module's own internals, which the attribution calls to time the
 * native crossing without the JavaScript that wraps it. Imported by the path
 * the package link resolves to, so they are the instances the package's
 * classes use: `native` is the addon those classes installed.
 */
interface Internals {
  readonly native: {
    connectionRun(handle: unknown, terminal: "all", sql: string, params: readonly unknown[], tagged: boolean, token: unknown): Promise<[string[], unknown[][], string[]]>;
    connectionStream(handle: unknown, sql: string, params: readonly unknown[], token: unknown): Promise<unknown>;
    streamNext(stream: unknown, tagged: boolean, token: unknown): Promise<[unknown[][], string[] | undefined, string[] | undefined] | null>;
    streamClose(stream: unknown): Promise<void>;
  };
  readonly row: (names: string[], values: unknown[]) => Record<string, unknown>;
  readonly connectionHandle: (connection: Connection) => unknown;
}

async function internals(): Promise<Internals> {
  const lib = (file: string) => pathToFileURL(join(checkout, "pgorm-napi", "lib", file)).href;
  const operations = (await import(lib("operations.js"))) as Pick<Internals, "native" | "row">;
  const connections = (await import(lib("connections.js"))) as Pick<Internals, "connectionHandle">;
  if (operations.native === undefined) {
    throw new Error("pgorm-napi's lib/operations.js loaded as a second instance, not the one the package installed its addon in");
  }
  return { native: operations.native, row: operations.row, connectionHandle: connections.connectionHandle };
}

/** Rows as the native call returns them, keyed by column for the checks. */
function keyed([names, rows]: [string[], unknown[][], string[]]): Record<string, unknown>[] {
  return rows.map((values) => {
    const out: Record<string, unknown> = {};
    names.forEach((name, at) => (out[name] = values[at]));
    return out;
  });
}

async function streamed<T>(rows: AsyncIterable<T>): Promise<T[]> {
  const out: T[] = [];
  for await (const row of rows) out.push(row);
  return out;
}

function preparedOf(connection: Connection) {
  return async () => preparedRows(await connection.query(PREPARED_SQL));
}

function attribution(connection: Connection, parts: Internals): (count: number) => Record<string, Operation> {
  const handle = parts.connectionHandle(connection);
  const nativeAll = (sql: string, count: number) => parts.native.connectionRun(handle, "all", sql, [count], false, undefined);
  return (count) => {
    // Fetched once, outside the timed operations that only build objects from it.
    let rowsResult: [string[], unknown[][], string[]] | undefined;
    let wideResult: [string[], unknown[][], string[]] | undefined;
    const fetched = async () => {
      rowsResult ??= await nativeAll(ROWS, count);
      wideResult ??= await nativeAll(WIDE, count);
      return [rowsResult, wideResult] as const;
    };
    const objects = (pick: 0 | 1) => async () => {
      const [names, rows] = (await fetched())[pick];
      return rows.map((values) => parts.row(names, values));
    };
    const loop = (pick: 0 | 1) => async () => {
      const [names, rows] = (await fetched())[pick];
      return rows.map((values) => {
        const out: Record<string, unknown> = {};
        for (let at = 0; at < names.length; at++) out[names[at] as string] = values[at];
        return out;
      });
    };
    return {
      native_rows_all: op(() => nativeAll(ROWS, count), (result) => checkRows(keyed(result), count, "native rows")),
      native_wide_all: op(() => nativeAll(WIDE, count), (result) => checkWide(keyed(result), count, "native wide")),
      native_rows_stream: op(
        async (): Promise<[string[], unknown[][], string[]]> => {
          const stream = await parts.native.connectionStream(handle, ROWS, [count], undefined);
          const rows: unknown[][] = [];
          let names: string[] = [];
          try {
            for (;;) {
              const batch = await parts.native.streamNext(stream, false, undefined);
              if (batch === null) break;
              if (batch[1]) names = batch[1];
              rows.push(...batch[0]);
            }
          } finally {
            await parts.native.streamClose(stream);
          }
          return [names, rows, []];
        },
        (result) => checkRows(keyed(result), count, "native stream"),
      ),
      row_objects: op(objects(0), (rows) => checkRows(rows, count, "row objects")),
      wide_row_objects: op(objects(1), (rows) => checkWide(rows, count, "wide row objects")),
      loop_objects: op(loop(0), (rows) => checkRows(rows, count, "loop objects")),
      wide_loop_objects: op(loop(1), (rows) => checkWide(rows, count, "wide loop objects")),
    };
  };
}

function rawSession(connection: Connection, pool: Pool, parts: Internals): Session {
  return {
    fixed: {
      scalar: {
        run: async () => (await connection.one(SCALAR, [SCALAR_VALUE])).int8,
        check: (value) => checkInt8(value, SCALAR_VALUE, "scalar"),
      },
      params16: {
        run: async () => (await connection.one(PARAMS16, Array.from({ length: 16 }, () => SCALAR_VALUE)))["?column?"],
        check: (value) => checkInt8(value, PARAMS16_SUM, "params16"),
      },
      model_lookup: {
        run: () => connection.one(LOOKUP, [LOOKUP_ID]),
        check: (row) => checkRow(row, LOOKUP_ID, "model_lookup"),
      },
      update16: {
        run: () => connection.execute(UPDATE16, [LOOKUP_ID, PAYLOAD, ...UPDATE_VALUES]),
        check: (count) => checkOneRow(count, "update16"),
      },
      transaction: {
        run: () => connection.transaction(async (tx) => (await tx.one(SCALAR, [SCALAR_VALUE])).int8),
        check: (value) => checkInt8(value, SCALAR_VALUE, "transaction"),
      },
    },
    counted: {
      rows_all: (count) => ({ run: () => connection.query(ROWS, [count]), check: (rows) => checkRows(rows, count, "rows_all") }),
      rows_stream: (count) => ({
        run: () => streamed(connection.stream(ROWS, [count])),
        check: (rows) => checkRows(rows, count, "rows_stream"),
      }),
      wide_all: (count) => ({ run: () => connection.query(WIDE, [count]), check: (rows) => checkWide(rows, count, "wide_all") }),
      column: (name, count) => ({
        run: () => connection.query(`SELECT ${name} FROM bench_wide ORDER BY id LIMIT $1`, [count]),
        check: (rows) => checkWideColumn(rows, name, count, `column ${name}`),
      }),
      texts: (width) => ({ run: () => connection.query(TEXTS, [width]), check: (rows) => checkTexts(rows, width, `texts ${width}`) }),
    },
    attribution: attribution(connection, parts),
    prepared: preparedOf(connection),
    async close() {
      await connection.close();
      await pool.close();
    },
  };
}

function ormSession(connection: Connection, pool: Pool): Session {
  return {
    fixed: {
      model_lookup: { run: () => lookup().one(connection), check: (row) => checkRow(row, LOOKUP_ID, "model_lookup") },
      update16: { run: () => update().execute(connection), check: (count) => checkOneRow(count, "update16") },
    },
    counted: {
      rows_all: (count) => ({ run: () => rowsQuery(count).all(connection), check: (rows) => checkRows(rows, count, "rows_all") }),
      rows_stream: (count) => ({
        run: () => streamed(rowsQuery(count).stream(connection)),
        check: (rows) => checkRows(rows, count, "rows_stream"),
      }),
      wide_all: (count) => ({ run: () => wideQuery(count).all(connection), check: (rows) => checkWide(rows, count, "wide_all") }),
    },
    prepared: preparedOf(connection),
    async close() {
      await connection.close();
      await pool.close();
    },
  };
}

async function held(dsn: string): Promise<[Pool, Connection]> {
  const pool = new Pool(dsn, { maxSize: 1 });
  try {
    return [pool, await pool.acquire()];
  } catch (error) {
    await pool.close();
    throw error;
  }
}

const RAW_ONLY = "the model API has no statement of its own for this; pgorm's raw path is pgorm/raw";

export function pgormSides(): Side[] {
  return [
    {
      name: "pgorm/raw",
      library: "pgorm",
      absent: {},
      sql: { scalar: SCALAR, params16: PARAMS16, model_lookup: LOOKUP, update16: UPDATE16, transaction: SCALAR, rows_all: ROWS, rows_stream: ROWS, wide_all: WIDE, concurrent_pool: SCALAR },
      async open(dsn) {
        const parts = await internals();
        const [pool, connection] = await held(dsn);
        return rawSession(connection, pool, parts);
      },
      poolVariants: ["recycle_fast"],
      async openPool(dsn: string, options: Options, variant?: string) {
        // The binding's default recycling is "verified", an empty query on every acquire; Rust's connect() defaults to Fast.
        const pool = new Pool(dsn, variant === "recycle_fast" ? { maxSize: options.poolSize, recycle: "fast" } : { maxSize: options.poolSize });
        return {
          operation: {
            run: async () => (await pool.one(SCALAR, [SCALAR_VALUE])).int8,
            check: (value: unknown) => checkInt8(value, SCALAR_VALUE, "concurrent_pool"),
          },
          close: () => pool.close(),
        };
      },
    },
    {
      name: "pgorm/orm",
      library: "pgorm",
      absent: { scalar: RAW_ONLY, params16: RAW_ONLY, transaction: RAW_ONLY, concurrent_pool: RAW_ONLY },
      sql: {
        model_lookup: lookup().inspect().sql,
        update16: update().inspect().sql,
        rows_all: rowsQuery(128).inspect().sql,
        rows_stream: rowsQuery(128).inspect().sql,
        wide_all: wideQuery(128).inspect().sql,
      },
      async open(dsn) {
        const [pool, connection] = await held(dsn);
        return ormSession(connection, pool);
      },
    },
  ];
}
