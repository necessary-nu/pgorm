import type { WideColumn } from "./fixture.ts";
import type { Operation } from "./measure.ts";
import type { Options } from "./options.ts";

/** The workloads one held connection runs, in the order a round runs them. */
export const SEQUENTIAL = [
  "scalar",
  "params16",
  "model_lookup",
  "update16",
  "transaction",
  "rows_all",
  "rows_stream",
  "wide_all",
] as const;
export type Sequential = (typeof SEQUENTIAL)[number];
export const WORKLOADS = [...SEQUENTIAL, "concurrent_pool"] as const;
export type Workload = (typeof WORKLOADS)[number];

/** A row of `pg_prepared_statements`, read on a side's own session. */
export interface Prepared {
  readonly name: string;
  readonly statement: string;
  readonly generic_plans: number;
  readonly custom_plans: number;
}

/** The workloads whose row count is a parameter, and the attribution's per-column and per-width reads. */
export interface Counted {
  readonly rows_all: (count: number) => Operation;
  readonly rows_stream?: (count: number) => Operation;
  readonly wide_all: (count: number) => Operation;
  readonly column?: (column: WideColumn, count: number) => Operation;
  readonly texts?: (width: number) => Operation;
}

/** A side's operations on the one connection it holds. */
export interface Session {
  readonly fixed: Partial<Record<Exclude<Sequential, "rows_all" | "rows_stream" | "wide_all">, Operation>>;
  readonly counted: Counted;
  /** Operations the attribution adds, beyond the workloads, by name. */
  readonly attribution?: (count: number) => Record<string, Operation>;
  /** The statements the server holds prepared for this session. */
  prepared(): Promise<Prepared[]>;
  close(): Promise<void>;
}

export interface PoolSession {
  readonly operation: Operation;
  close(): Promise<void>;
}

export interface Side {
  readonly name: string;
  /** The library a user of this side writes against. */
  readonly library: "pgorm" | "drizzle" | "driver";
  /** Why each workload this side does not run is n/a. */
  readonly absent: Partial<Record<Workload, string>>;
  /** The SQL each workload sends, as the side's own API renders it, `$N` placeholders and all. */
  readonly sql: Partial<Record<Workload, string>>;
  open(dsn: string, options: Options): Promise<Session>;
  /** The side's pool, built as the side's defaults build it unless `variant` names one of {@link poolVariants}. */
  openPool?(dsn: string, options: Options, variant?: string): Promise<PoolSession>;
  /** Pool configurations the attribution measures beside the default. */
  readonly poolVariants?: readonly string[];
}

export const PREPARED_SQL =
  "SELECT name, statement, generic_plans::int4 AS generic_plans, custom_plans::int4 AS custom_plans FROM pg_prepared_statements ORDER BY name";

export function preparedRows(rows: readonly Record<string, unknown>[]): Prepared[] {
  return rows
    .map((row) => ({
      name: String(row.name),
      statement: String(row.statement),
      generic_plans: Number(row.generic_plans),
      custom_plans: Number(row.custom_plans),
    }))
    .filter((row) => !row.statement.includes("pg_prepared_statements"));
}
