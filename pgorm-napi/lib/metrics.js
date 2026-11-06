// @ts-check

// pgorm's metrics layer: a pool that reports each statement, acquisition and
// transaction to a collector the program writes. An `InstrumentedPool` is
// pgorm's `InstrumentedPool` over the pool it wraps, so the timing, and which
// hook hears what, are pgorm's own. Each hook runs on the JavaScript thread,
// and the statement it reports on settles once the hook has: a plain hook
// once it returns, one returning a promise once that settles.
// [spec:pgorm:req:bindings.metrics]

import process from "node:process";

import { Pool, poolHandle } from "./connections.js";
import { native, TRUSTED } from "./operations.js";

/** @typedef {import("./index.d.ts").MetricsCollector} MetricsCollector */

/** The seven hooks, in pgorm's order. */
const HOOKS = /** @type {const} */ ([
  "recordQuerySuccess",
  "recordQueryError",
  "recordConnectionAcquired",
  "recordConnectionError",
  "recordTransactionBegin",
  "recordTransactionCommit",
  "recordTransactionRollback",
]);

/**
 * A statement's fingerprint: libpg_query's hash of its shape, which
 * `pg_stat_statements` groups by too. Its string is the 16 hex digits Rust's
 * writes, and `value` the same number.
 */
export class QueryFingerprint {
  /** @type {bigint} */
  #value;

  /**
   * @param {bigint} value
   * @param {symbol} trusted
   */
  constructor(value, trusted) {
    if (trusted !== TRUSTED) throw new TypeError("a QueryFingerprint comes from a QueryContext");
    this.#value = value;
  }

  /** @returns {bigint} */
  get value() {
    return this.#value;
  }

  toString() {
    return this.#value.toString(16).padStart(16, "0");
  }

  /**
   * Whether `other` is the same fingerprint.
   *
   * @param {unknown} other
   * @returns {boolean}
   */
  equals(other) {
    return other instanceof QueryFingerprint && other.value === this.#value;
  }
}

/**
 * What a query hook is told about the statement it reports on: the operation
 * that ran it, its SQL, and its fingerprint, parsed the first time the
 * process asks for one of this text.
 */
export class QueryContext {
  /** @type {string} */
  #operation;
  /** @type {string | null} */
  #sql;

  /**
   * @param {string} operation
   * @param {string | null} [sql]
   */
  constructor(operation, sql = null) {
    if (typeof operation !== "string") throw new TypeError("operation is a string");
    if (sql !== null && typeof sql !== "string") throw new TypeError("sql is a string or null");
    this.#operation = operation;
    this.#sql = sql;
  }

  /** @returns {string} */
  get operation() {
    return this.#operation;
  }

  /** @returns {string | null} */
  get sql() {
    return this.#sql;
  }

  /** @returns {QueryFingerprint | null} */
  get fingerprint() {
    const value = native.queryFingerprint(this.#operation, this.#sql);
    return value === null ? null : new QueryFingerprint(value, TRUSTED);
  }
}

/**
 * A collector whose every hook does nothing. A collector that cares about
 * some hooks only extends it and overrides those; a hook left as this class
 * wrote it is never run, so it costs the statement nothing beyond pgorm's own
 * timing.
 *
 * @implements {MetricsCollector}
 */
export class NoOpMetrics {
  /**
   * @param {QueryContext} _query
   * @param {number} _duration
   * @param {number | null} _rows
   * @returns {void | Promise<void>}
   */
  recordQuerySuccess(_query, _duration, _rows) {}

  /**
   * @param {QueryContext} _query
   * @param {number} _duration
   * @param {Error} _error
   * @returns {void | Promise<void>}
   */
  recordQueryError(_query, _duration, _error) {}

  /**
   * @param {number} _duration
   * @returns {void | Promise<void>}
   */
  recordConnectionAcquired(_duration) {}

  /**
   * @param {number} _duration
   * @param {Error} _error
   * @returns {void | Promise<void>}
   */
  recordConnectionError(_duration, _error) {}

  /**
   * @param {number} _duration
   * @returns {void | Promise<void>}
   */
  recordTransactionBegin(_duration) {}

  /**
   * @param {number} _duration
   * @returns {void | Promise<void>}
   */
  recordTransactionCommit(_duration) {}

  /**
   * @param {number} _duration
   * @returns {void | Promise<void>}
   */
  recordTransactionRollback(_duration) {}
}

/** @param {number} duration */
const elapsed = (duration) => `${duration.toFixed(3)}ms`;

/** @param {QueryContext} query */
function identity(query) {
  const fingerprint = query.fingerprint;
  return fingerprint === null ? "" : ` [${fingerprint}]`;
}

/**
 * A collector writing each report to `logger`, `console` unless another is
 * given, at the levels Rust's `LoggingMetrics` writes its `tracing` events
 * at: `debug` for a successful query, a connection, a transaction's begin and
 * its commit; `warn` for a failed query and a rollback; `error` for a failure
 * to get a connection. A query's message ends with its fingerprint when it
 * has one.
 *
 * @implements {MetricsCollector}
 */
export class LoggingMetrics {
  /** @type {import("./index.d.ts").MetricsLogger} */
  #logger;

  /** @param {import("./index.d.ts").MetricsLogger} [logger] */
  constructor(logger = console) {
    if (typeof logger !== "object" || logger === null) throw new TypeError("logger is an object");
    for (const level of ["debug", "warn", "error"]) {
      if (typeof Reflect.get(logger, level) !== "function") throw new TypeError(`logger has a ${level} method`);
    }
    this.#logger = logger;
  }

  /**
   * @param {QueryContext} query
   * @param {number} duration
   * @param {number | null} rows
   */
  recordQuerySuccess(query, duration, rows) {
    const counted = rows === null ? "" : `, ${rows} rows`;
    this.#logger.debug(`Query ${query.operation} succeeded in ${elapsed(duration)}${counted}${identity(query)}`);
  }

  /**
   * @param {QueryContext} query
   * @param {number} duration
   * @param {Error} error
   */
  recordQueryError(query, duration, error) {
    this.#logger.warn(`Query ${query.operation} failed after ${elapsed(duration)}: ${error.message}${identity(query)}`);
  }

  /** @param {number} duration */
  recordConnectionAcquired(duration) {
    this.#logger.debug(`Connection acquired in ${elapsed(duration)}`);
  }

  /**
   * @param {number} duration
   * @param {Error} error
   */
  recordConnectionError(duration, error) {
    this.#logger.error(`Connection failed after ${elapsed(duration)}: ${error.message}`);
  }

  /** @param {number} duration */
  recordTransactionBegin(duration) {
    this.#logger.debug(`Transaction began in ${elapsed(duration)}`);
  }

  /** @param {number} duration */
  recordTransactionCommit(duration) {
    this.#logger.debug(`Transaction committed in ${elapsed(duration)}`);
  }

  /** @param {number} duration */
  recordTransactionRollback(duration) {
    this.#logger.warn(`Transaction rolled back in ${elapsed(duration)}`);
  }
}

/**
 * Report a hook's failure as a process warning, named `MetricsWarning`, the
 * hook's error its `cause`.
 *
 * @param {string} name
 * @param {unknown} error
 */
function failed(name, error) {
  const warning = new Error(`pgorm's metrics hook ${name} failed`, { cause: error });
  warning.name = "MetricsWarning";
  process.emitWarning(warning);
}

/**
 * An operation as JavaScript spells a closed choice: `query_all` is
 * `queryAll`.
 *
 * @param {string} name
 */
const camel = (name) => name.replace(/_([a-z])/g, (_, letter) => letter.toUpperCase());

/**
 * Run one hook on the JavaScript thread and call `done` once it has
 * finished, which lets the statement it reports on settle. Its failure is
 * reported and never reaches that statement.
 *
 * @param {string} name
 * @param {(...args: any[]) => unknown} hook
 * @param {[string, string | null] | null} query
 * @param {unknown[]} args
 * @param {() => void} done
 */
function dispatch(name, hook, query, args, done) {
  /** @type {unknown} */
  let result;
  try {
    result = query === null ? hook(...args) : hook(new QueryContext(camel(query[0]), query[1]), ...args);
  } catch (error) {
    try {
      failed(name, error);
    } finally {
      done();
    }
    return;
  }
  if (typeof result === "object" && result !== null && typeof Reflect.get(result, "then") === "function") {
    Promise.resolve(result).then(
      () => done(),
      (error) => {
        try {
          failed(name, error);
        } finally {
          done();
        }
      },
    );
    return;
  }
  done();
}

/**
 * The collector's seven hooks in pgorm's order, each bound to it, or `null`
 * for each it left as {@link NoOpMetrics} wrote it.
 *
 * @param {unknown} metrics
 * @returns {(Function | null)[]}
 */
function hooksOf(metrics) {
  if (typeof metrics !== "object" || metrics === null) throw new TypeError("a metrics collector is an object");
  return HOOKS.map((name) => {
    const hook = Reflect.get(metrics, name);
    if (typeof hook !== "function") throw new TypeError(`a metrics collector has a ${name} function`);
    return hook === NoOpMetrics.prototype[name] ? null : hook.bind(metrics);
  });
}

/**
 * `pool`, reporting to `metrics` what runs through it. It is the same pool:
 * its connections, its size and its closing are the wrapped pool's, and
 * closing either closes both. A statement, a registered entity's or a model's
 * query, a pipeline, a stream, a COPY's start and a notification run on it, or
 * on a connection it lends, or in a transaction or savepoint on one, are
 * reported, and so is each acquisition and each transaction's begin and end.
 * `inner` runs the same work unreported, and a listener is not reported at
 * all.
 *
 * @template {MetricsCollector} M
 */
export class InstrumentedPool extends Pool {
  /** @type {Pool} */
  #inner;
  /** @type {M} */
  #metrics;

  /**
   * @param {Pool} pool
   * @param {M} metrics
   */
  constructor(pool, metrics) {
    if (!(pool instanceof Pool)) throw new TypeError("an InstrumentedPool wraps a Pool");
    super(TRUSTED, native.poolInstrument(poolHandle(pool), hooksOf(metrics), dispatch));
    this.#inner = pool;
    this.#metrics = metrics;
  }

  /**
   * The pool this one wraps, whose work is not reported.
   *
   * @returns {Pool}
   */
  get inner() {
    return this.#inner;
  }

  /**
   * The collector this pool reports to.
   *
   * @returns {M}
   */
  get metrics() {
    return this.#metrics;
  }
}
