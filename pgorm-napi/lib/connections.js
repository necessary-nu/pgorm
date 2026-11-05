// @ts-check

// Pools, connections and transactions. Each holds a native handle and
// releases what it holds when closed — `close()`, `await using`, the end of a
// callback — never by waiting for garbage collection.
// [spec:pgorm:req:bindings.connections+2]
// [spec:pgorm:req:napi.lifecycle+2]
// [spec:pgorm:req:bindings.results+2]
// [spec:pgorm:req:napi.raw]
// [spec:pgorm:req:bindings.transactions]
// [spec:pgorm:req:napi.transactions+3]
// [spec:pgorm:req:bindings.copy]
// [spec:pgorm:req:bindings.listen+2]

import { statementArgs } from "./builder.js";
import { COPY, CopyRunner } from "./copy.js";
import { Listener, LISTENER, listenerOptions, notifyJob } from "./listen.js";
import { abortable, LATE, native, queryOptions, row, scoped, signalOf, TRUSTED } from "./operations.js";
import { RowStream } from "./streams.js";

/** The statement runner each class implements, keyed privately. */
const RUN = Symbol("run");

/** The job runner each class implements, keyed privately. */
const JOB = Symbol("job");

/**
 * What a pool, a connection and a transaction run statements through: bound
 * SQL, its result decoded to rows. Every method that does I/O is an async
 * function, so an argument the addon refuses synchronously rejects its
 * promise rather than throwing at the call site. COPY's `copyIn` and
 * `copyOut` come from {@link CopyRunner}, each class opening a COPY in the
 * addon on what it holds.
 * [spec:pgorm:req:napi.promises]
 */
export class Queryable extends CopyRunner {
  /**
   * @param {"execute" | "all" | "one" | "optional"} _terminal
   * @param {unknown} _sql
   * @param {readonly unknown[]} _params
   * @param {boolean} _tagged
   * @param {unknown} _token
   * @returns {Promise<any>}
   */
  [RUN](_terminal, _sql, _params, _tagged, _token) {
    return Promise.reject(new TypeError("Queryable is implemented by Pool, Connection and Transaction"));
  }

  /**
   * @param {unknown} _job
   * @param {unknown} _token
   * @returns {Promise<any>}
   */
  [JOB](_job, _token) {
    return Promise.reject(new TypeError("Queryable is implemented by Pool, Connection and Transaction"));
  }

  /**
   * @param {"all" | "one" | "optional"} terminal
   * @param {unknown} statement
   * @param {unknown} params
   * @param {unknown} options
   * @returns {Promise<Record<string, unknown>[]>}
   */
  async #rows(terminal, statement, params, options) {
    const [sql, values, rest] = statementArgs(statement, params, options);
    const { tagged, signal } = queryOptions(rest);
    const [names, rows] = await abortable(
      signal,
      (token) => this[RUN](terminal, sql, values, tagged, token),
      () => this[LATE](),
    );
    return rows.map((/** @type {unknown[]} */ values) => row(names, values));
  }

  /**
   * Run a statement — SQL text and its parameters, or a statement a builder
   * made and its options — and resolve with the number of rows it affected.
   *
   * @param {unknown} statement
   * @param {unknown} [params]
   * @param {unknown} [options]
   * @returns {Promise<number>}
   */
  async execute(statement, params, options) {
    const [sql, values, rest] = statementArgs(statement, params, options);
    const { signal } = queryOptions(rest);
    return await abortable(signal, (token) => this[RUN]("execute", sql, values, false, token), () => this[LATE]());
  }

  /**
   * Every row.
   *
   * @param {unknown} statement
   * @param {unknown} [params]
   * @param {unknown} [options]
   */
  async query(statement, params, options) {
    return await this.#rows("all", statement, params, options);
  }

  /**
   * Exactly one row; any other count is a `DecodeError`.
   *
   * @param {unknown} statement
   * @param {unknown} [params]
   * @param {unknown} [options]
   */
  async one(statement, params, options) {
    return (await this.#rows("one", statement, params, options))[0];
  }

  /**
   * At most one row, or `null`; more is a `DecodeError`.
   *
   * @param {unknown} statement
   * @param {unknown} [params]
   * @param {unknown} [options]
   */
  async optional(statement, params, options) {
    return (await this.#rows("optional", statement, params, options))[0] ?? null;
  }

  /**
   * Send `payload` on `channel` through `SELECT pg_notify($1, $2)`, both
   * bound. In a transaction it is delivered when the transaction commits, and
   * never if it rolls back.
   *
   * @param {string} channel
   * @param {string} payload
   * @param {{ signal?: AbortSignal }} [options]
   * @returns {Promise<void>}
   */
  async notify(channel, payload, options = {}) {
    const job = notifyJob(channel, payload);
    const { signal } = queryOptions(options);
    await abortable(signal, (token) => this[JOB](job, token), () => this[LATE]());
  }
}

/**
 * Run a built statement and resolve with its result as the addon gives it,
 * `[names, rows, kinds]` — each row an array of values in column order, and
 * each column's kind — for a model to decode against its declaration.
 * [spec:pgorm:req:bindings.model-records+1]
 *
 * @param {unknown} db
 * @param {"execute" | "all" | "one" | "optional"} terminal
 * @param {import("./builder.js").Handle} statement
 * @param {unknown} options
 * @returns {Promise<any>}
 */
export async function runStatement(db, terminal, statement, options) {
  if (!(db instanceof Queryable)) {
    throw new TypeError("a model's statement runs on a Pool, a Connection or a Transaction");
  }
  const [handle, , rest] = statementArgs(statement, undefined, options);
  const { signal } = queryOptions(rest);
  return await abortable(signal, (token) => db[RUN](terminal, handle, [], false, token), () => db[LATE]());
}

/**
 * The rows of a disjoint read's `statement`, the native half of a `Select`,
 * as a stream on a pool's connection or a connection, each row read by
 * `disjoint`'s codec.
 * [spec:pgorm:req:bindings.disjoint+2]
 *
 * @param {unknown} db
 * @param {unknown} statement
 * @param {unknown} disjoint
 * @param {unknown} options
 * @returns {RowStream}
 */
export function disjointStream(db, statement, disjoint, options) {
  if (db instanceof Pool) {
    return new RowStream(async (token) => {
      const connection = new Connection(await native.poolAcquire(poolHandle(db), token), TRUSTED);
      try {
        const handle = await native.connectionStream(connectionHandle(connection), statement, [], token, disjoint);
        return [handle, () => connection.close()];
      } catch (error) {
        await connection.close();
        throw error;
      }
    }, options ?? {}, TRUSTED);
  }
  if (db instanceof Connection) {
    return new RowStream(
      async (token) => [await native.connectionStream(connectionHandle(db), statement, [], token, disjoint), undefined],
      options ?? {},
      TRUSTED,
    );
  }
  throw new TypeError("a disjoint read's rows stream from a Pool or a Connection");
}

/**
 * Run a registered entity's job — one of pgorm's own terminals, built by the
 * addon — on a pool's connection, a connection or a transaction, with the
 * operation a statement gets around it: the busy refusal, the abort, the
 * discard of a connection an abort left in an unknown state.
 * [spec:pgorm:req:bindings.entities+5]
 * [spec:pgorm:req:bindings.registered-pools]
 *
 * @param {unknown} db
 * @param {unknown} job
 * @param {unknown} options
 * @returns {Promise<any>}
 */
export async function runJob(db, job, options) {
  if (!(db instanceof Pool || db instanceof Connection || db instanceof Transaction)) {
    throw new TypeError("a registered entity runs on a Pool, a Connection or a Transaction");
  }
  const { signal } = queryOptions(options ?? {});
  return await abortable(signal, (token) => db[JOB](job, token), () => db[LATE]());
}

/** @type {(connection: Connection) => unknown} */
let connectionHandle;

/** @type {(pool: Pool) => unknown} */
let poolHandle;

/**
 * The native handle of `pool`, which an {@link InstrumentedPool} is built
 * around.
 *
 * @param {Pool} pool
 * @returns {unknown}
 */
export function nativePool(pool) {
  return poolHandle(pool);
}

/**
 * A pool of connections to one server, opened as they are first needed. TLS
 * verifies the server's certificate and host name unless the connection
 * string says `sslmode=disable` or `tls` is `"disable"`.
 */
export class Pool extends Queryable {
  /** @type {unknown} */
  #native;

  static {
    poolHandle = (pool) => pool.#native;
  }

  /**
   * A pool built from a connection string and its options, or, for
   * {@link InstrumentedPool} alone, around the native handle the addon made.
   *
   * @param {string | symbol} dsn
   * @param {import("./index.d.ts").PoolOptions | unknown} [options]
   */
  constructor(dsn, options = {}) {
    super();
    if (dsn === TRUSTED) {
      this.#native = options;
      return;
    }
    if (typeof dsn !== "string") throw new TypeError("dsn is a connection string");
    if (typeof options !== "object" || options === null) throw new TypeError("options is an object");
    this.#native = native.poolNew(dsn, options);
  }

  /** @returns {boolean} */
  get closed() {
    return native.poolClosed(this.#native);
  }

  /** @returns {import("./index.d.ts").PoolStatus} */
  status() {
    return native.poolStatus(this.#native);
  }

  clearStatementCaches() {
    native.poolClearStatementCaches(this.#native);
  }

  /**
   * A connection of the caller's own until it closes it.
   *
   * @param {{ signal?: AbortSignal }} [options]
   * @returns {Promise<Connection>}
   */
  async acquire(options = {}) {
    const { signal } = queryOptions(options);
    const handle = await abortable(
      signal,
      (token) => native.poolAcquire(this.#native, token),
      (acquired) => native.connectionClose(acquired),
    );
    return new Connection(handle, TRUSTED);
  }

  /**
   * Run `use` with a connection, closed when it settles.
   *
   * @template T
   * @param {(connection: Connection) => Promise<T>} use
   * @param {{ signal?: AbortSignal }} [options]
   * @returns {Promise<T>}
   */
  async connection(use, options = {}) {
    const connection = await this.acquire(options);
    try {
      return await use(connection);
    } finally {
      await connection.close();
    }
  }

  /**
   * Run `use` in a transaction on a connection of its own: committed when
   * `use` resolves, rolled back when it throws.
   *
   * @template T
   * @param {(transaction: Transaction) => Promise<T>} use
   * @param {import("./index.d.ts").TransactionOptions} [options]
   * @returns {Promise<T>}
   */
  async transaction(use, options = {}) {
    return await this.connection((connection) => connection.transaction(use, options), {
      signal: options.signal,
    });
  }

  /**
   * The rows of `sql`, pulled one at a time over a connection the stream
   * holds until its last row or until it is closed.
   *
   * @param {unknown} statement
   * @param {unknown} [params]
   * @param {unknown} [options]
   * @returns {RowStream}
   */
  stream(statement, params, options) {
    const [sql, values, rest] = statementArgs(statement, params, options);
    const pool = this;
    return new RowStream(async (token) => {
      const connection = new Connection(await native.poolAcquire(pool.#native, token), TRUSTED);
      try {
        const handle = await native.connectionStream(connectionHandle(connection), sql, values, token);
        return [handle, () => connection.close()];
      } catch (error) {
        await connection.close();
        throw error;
      }
    }, rest, TRUSTED);
  }

  /**
   * Whether the server answers, on a connection of its own.
   *
   * @param {{ signal?: AbortSignal }} [options]
   * @returns {Promise<boolean>}
   */
  async ping(options = {}) {
    return await this.connection((connection) => connection.ping(options), options);
  }

  /**
   * A listener on a connection of its own, opened with the pool's
   * configuration and closed with the pool, holding at most `capacity`
   * notifications unread.
   *
   * @param {import("./index.d.ts").ListenerOptions} [options]
   * @returns {Promise<Listener>}
   */
  async listener(options = {}) {
    const { capacity, signal } = listenerOptions(options);
    const handle = await abortable(
      signal,
      (token) => native.poolListener(this.#native, capacity, token),
      (opened) => native.listenerClose(opened),
    );
    return new Listener(handle, LISTENER);
  }

  /**
   * Refuse new work, cancel what runs, and resolve once every connection is
   * released.
   *
   * @returns {Promise<void>}
   */
  async close() {
    await native.poolClose(this.#native);
  }

  [Symbol.asyncDispose]() {
    return this.close();
  }

  /**
   * @override
   * @param {"execute" | "all" | "one" | "optional"} terminal
   * @param {unknown} sql
   * @param {readonly unknown[]} params
   * @param {boolean} tagged
   * @param {unknown} token
   */
  [RUN](terminal, sql, params, tagged, token) {
    return native.poolRun(this.#native, terminal, sql, params, tagged, token);
  }

  /**
   * @override
   * @param {unknown} job
   * @param {unknown} token
   */
  [JOB](job, token) {
    return native.poolJob(this.#native, job, token);
  }

  /**
   * @override
   * @param {"in" | "out"} direction
   * @param {unknown} copy
   * @param {unknown} token
   * @param {boolean} rows
   */
  [COPY](direction, copy, token, rows) {
    return (direction === "in" ? native.poolCopyIn : native.poolCopyOut)(this.#native, copy, token, rows);
  }
}

/**
 * A pool whose server has answered.
 *
 * @param {string} dsn
 * @param {import("./index.d.ts").PoolOptions} [options]
 * @returns {Promise<Pool>}
 */
export async function connect(dsn, options = {}) {
  const pool = new Pool(dsn, options);
  try {
    await pool.ping();
  } catch (error) {
    await pool.close();
    throw error;
  }
  return pool;
}

/** One connection checked out of a pool; one operation at a time. */
export class Connection extends Queryable {
  /** @type {unknown} */
  #native;

  /**
   * @param {unknown} handle
   * @param {symbol} trusted
   */
  constructor(handle, trusted) {
    super();
    if (trusted !== TRUSTED) throw new TypeError("a Connection comes from pool.acquire()");
    this.#native = handle;
  }

  static {
    connectionHandle = (connection) => connection.#native;
  }

  /** @returns {boolean} */
  get closed() {
    return native.connectionClosed(this.#native);
  }

  /**
   * Run `use` in a transaction, which holds the connection until `use`
   * settles: committed when it resolves, rolled back when it throws.
   *
   * @template T
   * @param {(transaction: Transaction) => Promise<T>} use
   * @param {import("./index.d.ts").TransactionOptions} [options]
   * @returns {Promise<T>}
   */
  async transaction(use, options = {}) {
    const { mode = "default", isolation = null, signal } = options;
    const handle = await abortable(
      signalOf(signal),
      (token) => native.connectionBegin(this.#native, mode, isolation, token),
      (begun) => native.transactionAbort(begun),
    );
    return await scoped(handle, new Transaction(handle, this, TRUSTED), use);
  }

  /**
   * The rows of `sql`, pulled one at a time; the connection is the stream's
   * until its last row or until it is closed, and a stream closed early
   * discards it.
   *
   * @param {unknown} statement
   * @param {unknown} [params]
   * @param {unknown} [options]
   * @returns {RowStream}
   */
  stream(statement, params, options) {
    const [sql, values, rest] = statementArgs(statement, params, options);
    const connection = this;
    return new RowStream(
      async (token) => [await native.connectionStream(connection.#native, sql, values, token), undefined],
      rest,
      TRUSTED,
    );
  }

  /**
   * @param {{ signal?: AbortSignal }} [options]
   * @returns {Promise<boolean>}
   */
  async ping(options = {}) {
    const { signal } = queryOptions(options);
    return await abortable(signal, (token) => native.connectionPing(this.#native, token));
  }

  /**
   * Return the connection to its pool — or, if a transaction or stream still
   * holds it, end that and discard it — and resolve once it is released.
   *
   * @returns {Promise<void>}
   */
  async close() {
    await native.connectionClose(this.#native);
  }

  [Symbol.asyncDispose]() {
    return this.close();
  }

  /**
   * @override
   * @param {"execute" | "all" | "one" | "optional"} terminal
   * @param {unknown} sql
   * @param {readonly unknown[]} params
   * @param {boolean} tagged
   * @param {unknown} token
   */
  [RUN](terminal, sql, params, tagged, token) {
    return native.connectionRun(this.#native, terminal, sql, params, tagged, token);
  }

  /**
   * @override
   * @param {unknown} job
   * @param {unknown} token
   */
  [JOB](job, token) {
    return native.connectionJob(this.#native, job, token);
  }

  /**
   * @override
   * @param {"in" | "out"} direction
   * @param {unknown} copy
   * @param {unknown} token
   * @param {boolean} rows
   */
  [COPY](direction, copy, token, rows) {
    return (direction === "in" ? native.connectionCopyIn : native.connectionCopyOut)(this.#native, copy, token, rows);
  }
}

/**
 * A transaction, or a savepoint inside one, open for the callback
 * `transaction(fn)` runs and ended as that callback settles. While a savepoint
 * is open its parent refuses statements and other savepoints.
 */
export class Transaction extends Queryable {
  /** @type {unknown} */
  #native;
  /**
   * The connection or transaction this one borrows, kept reachable so that
   * collecting a parent nobody else holds never ends it under its child.
   *
   * @type {Connection | Transaction}
   */
  #parent;

  /**
   * @param {unknown} handle
   * @param {Connection | Transaction} parent
   * @param {symbol} trusted
   */
  constructor(handle, parent, trusted) {
    super();
    if (trusted !== TRUSTED) throw new TypeError("a Transaction comes from transaction()");
    this.#native = handle;
    this.#parent = parent;
  }

  /** @returns {Connection | Transaction} */
  get parent() {
    return this.#parent;
  }

  /** @returns {boolean} */
  get closed() {
    return native.transactionClosed(this.#native);
  }

  /**
   * End the whole transaction with nothing committed, discarding its
   * connection, as an abort the addon saw ends it: a statement aborted after
   * it ran leaves no write behind.
   *
   * @override
   * @returns {Promise<void>}
   */
  async [LATE]() {
    await native.transactionAbort(this.#native);
  }

  /**
   * Run `use` in a savepoint, which holds this transaction until `use`
   * settles: released when it resolves, rolled back to when it throws.
   *
   * @template T
   * @param {(transaction: Transaction) => Promise<T>} use
   * @param {{ signal?: AbortSignal }} [options]
   * @returns {Promise<T>}
   */
  async transaction(use, options = {}) {
    const { signal } = queryOptions(options);
    const handle = await abortable(
      signal,
      (token) => native.transactionBegin(this.#native, token),
      (begun) => native.transactionAbort(begun),
    );
    return await scoped(handle, new Transaction(handle, this, TRUSTED), use);
  }

  /**
   * @override
   * @param {"execute" | "all" | "one" | "optional"} terminal
   * @param {unknown} sql
   * @param {readonly unknown[]} params
   * @param {boolean} tagged
   * @param {unknown} token
   */
  [RUN](terminal, sql, params, tagged, token) {
    return native.transactionRun(this.#native, terminal, sql, params, tagged, token);
  }

  /**
   * @override
   * @param {unknown} job
   * @param {unknown} token
   */
  [JOB](job, token) {
    return native.transactionJob(this.#native, job, token);
  }

  /**
   * @override
   * @param {"in" | "out"} direction
   * @param {unknown} copy
   * @param {unknown} token
   * @param {boolean} rows
   */
  [COPY](direction, copy, token, rows) {
    return (direction === "in" ? native.transactionCopyIn : native.transactionCopyOut)(this.#native, copy, token, rows);
  }
}
