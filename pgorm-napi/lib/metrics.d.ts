// [spec:pgorm:req:bindings.metrics]
import type { PgormError, Pool } from "./index.d.ts";

/**
 * What an {@link InstrumentedPool} reports to: Rust's seven hooks, each a
 * plain function or one returning a promise, which the statement it reports
 * on waits for. `duration` is in milliseconds. `recordQueryError` and
 * `recordConnectionError` are given the error the caller is rejected with.
 * `recordTransactionRollback` hears every transaction or savepoint that ended
 * without committing, a failed `COMMIT` included.
 */
export interface MetricsCollector {
  recordQuerySuccess(query: QueryContext, duration: number, rows: number | null): void | Promise<void>;
  recordQueryError(query: QueryContext, duration: number, error: PgormError): void | Promise<void>;
  recordConnectionAcquired(duration: number): void | Promise<void>;
  recordConnectionError(duration: number, error: PgormError): void | Promise<void>;
  recordTransactionBegin(duration: number): void | Promise<void>;
  recordTransactionCommit(duration: number): void | Promise<void>;
  recordTransactionRollback(duration: number): void | Promise<void>;
}

/**
 * A collector whose every hook does nothing. A collector that cares about
 * some hooks only extends it and overrides those; a hook left as this class
 * wrote it is never run.
 */
export declare class NoOpMetrics implements MetricsCollector {
  recordQuerySuccess(query: QueryContext, duration: number, rows: number | null): void | Promise<void>;
  recordQueryError(query: QueryContext, duration: number, error: PgormError): void | Promise<void>;
  recordConnectionAcquired(duration: number): void | Promise<void>;
  recordConnectionError(duration: number, error: PgormError): void | Promise<void>;
  recordTransactionBegin(duration: number): void | Promise<void>;
  recordTransactionCommit(duration: number): void | Promise<void>;
  recordTransactionRollback(duration: number): void | Promise<void>;
}

/** Where {@link LoggingMetrics} writes: `console`, or a logger of the same three methods. */
export interface MetricsLogger {
  debug(message: string): void;
  warn(message: string): void;
  error(message: string): void;
}

/**
 * A collector writing each report to its logger at the levels Rust's
 * `LoggingMetrics` writes its `tracing` events at: `debug` for a successful
 * query, a connection, a transaction's begin and its commit; `warn` for a
 * failed query and a rollback; `error` for a failure to get a connection.
 */
export declare class LoggingMetrics implements MetricsCollector {
  constructor(logger?: MetricsLogger);
  recordQuerySuccess(query: QueryContext, duration: number, rows: number | null): void;
  recordQueryError(query: QueryContext, duration: number, error: PgormError): void;
  recordConnectionAcquired(duration: number): void;
  recordConnectionError(duration: number, error: PgormError): void;
  recordTransactionBegin(duration: number): void;
  recordTransactionCommit(duration: number): void;
  recordTransactionRollback(duration: number): void;
}

/**
 * What a query hook is told about the statement it reports on. `sql` is
 * `null` only for a failed `begin`, `commit` or `rollback`, which sent no
 * statement.
 */
export declare class QueryContext {
  constructor(operation: string, sql?: string | null);
  /**
   * The operation that ran it, Rust's name for it in camelCase: `"execute"`,
   * `"executeRaw"`, `"queryOne"`, `"queryOpt"`, `"queryAll"`, `"queryRaw"`,
   * `"batchExecute"`, `"copyIn"`, `"copyOut"`, or a failed `"begin"`,
   * `"commit"` or `"rollback"`.
   */
  readonly operation: string;
  readonly sql: string | null;
  /** The statement's fingerprint, or `null` when it has no statement or libpg_query refuses its text. */
  readonly fingerprint: QueryFingerprint | null;
}

/**
 * libpg_query's hash of a statement's shape, the identity
 * `pg_stat_statements` groups by: its string is 16 hex digits, and `value`
 * the same number.
 */
export declare class QueryFingerprint {
  private constructor();
  readonly value: bigint;
  toString(): string;
  equals(other: unknown): boolean;
}

/**
 * `pool`, reporting to `metrics` what runs through it, its connections' and
 * their transactions' work included. It is the same pool, closed with it;
 * `inner` runs the same work unreported.
 */
export declare class InstrumentedPool<M extends MetricsCollector = MetricsCollector> extends Pool {
  constructor(pool: Pool, metrics: M);
  /** The pool this one wraps, whose work is not reported. */
  readonly inner: Pool;
  /** The collector this pool reports to. */
  readonly metrics: M;
}
