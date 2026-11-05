// @ts-check

// COPY: the `COPY … FROM STDIN` and `COPY … TO STDOUT` statements, built over
// pgorm-query's, and the data flow `copyIn` and `copyOut` run them with, as
// bytes, and `copyInRows` and `copyOutRows`, as rows of values in the binary
// format. A COPY's format is named as it is made, and an option its format
// or its direction does not take is refused as it is called. COPY takes no
// bind parameters, so `inspect()` gives its SQL, every value in it a
// literal, and no values.
// [spec:pgorm:req:bindings.copy]
// [spec:pgorm:req:bindings.copy-rows+1]

import { arg, Builder, trusted, TRUSTED } from "./builder.js";
import { pump } from "./copy-feed.js";
import { CopyRowStream, pumpRows, rowSource } from "./copy-rows.js";
import { LifecycleError } from "./errors.js";
import { abortable, LATE, native, queryOptions, TRUSTED as HANDLE } from "./operations.js";

/**
 * @typedef {"text" | "csv" | "binary"} CopyFormat
 * @typedef {import("./schema.js").TableRef} TableRef
 * @typedef {import("./expressions.js").Expr | import("./conditions.js").Condition} Predicate
 * @typedef {{ rejectLimit?: number | bigint, logVerbosity?: "verbose" | "silent" }} CopyIgnoreOptions
 */

/**
 * What both directions of COPY share: the format, and every option either
 * takes. Each is offered on both classes, so that one the statement's
 * direction or format does not take is refused with a `ConstructionError`
 * as it is called, never a missing method; the declarations offer each class
 * only what it takes.
 */
class CopyStatement extends Builder {
  /** @type {CopyFormat} */
  #format;

  /**
   * @param {unknown} handle
   * @param {CopyFormat} format
   */
  constructor(handle, format) {
    super(handle);
    this.#format = format;
  }

  /** @returns {CopyFormat} */
  get format() {
    return this.#format;
  }

  /**
   * The statement of the receiver's class over `handle`.
   *
   * @param {unknown} handle
   * @returns {any}
   */
  #next(handle) {
    return this instanceof CopyFrom
      ? new CopyFrom(handle, this.#format, TRUSTED)
      : new CopyTo(handle, this.#format, TRUSTED);
  }

  /**
   * The columns each row supplies, in order, added to those already named; a
   * column left out takes its default.
   *
   * @param {readonly string[]} columns
   */
  columns(columns) {
    return this.#next(native.copyColumns(arg(this), columns));
  }

  /** `FREEZE`: write the rows already frozen. */
  freeze() {
    return this.#next(native.copyFreeze(arg(this)));
  }

  /**
   * Only the rows `predicate` holds for, ANDed to a condition already there.
   *
   * @param {Predicate} predicate
   */
  where(predicate) {
    return this.#next(native.copyWhere(arg(this), arg(predicate)));
  }

  /** @param {string} delimiter */
  delimiter(delimiter) {
    return this.#next(native.copyDelimiter(arg(this), delimiter));
  }

  /** @param {string} text */
  nullString(text) {
    return this.#next(native.copyNullString(arg(this), text));
  }

  /** @param {string} text */
  defaultString(text) {
    return this.#next(native.copyDefaultString(arg(this), text));
  }

  header() {
    return this.#next(native.copyHeader(arg(this)));
  }

  headerMatch() {
    return this.#next(native.copyHeaderMatch(arg(this)));
  }

  /** @param {string} encoding */
  encoding(encoding) {
    return this.#next(native.copyEncoding(arg(this), encoding));
  }

  /** @param {CopyIgnoreOptions} [options] */
  onErrorIgnore(options) {
    return this.#next(native.copyOnErrorIgnore(arg(this), options));
  }

  /** @param {string} quote */
  quote(quote) {
    return this.#next(native.copyQuote(arg(this), quote));
  }

  /** @param {string} escape */
  escape(escape) {
    return this.#next(native.copyEscape(arg(this), escape));
  }

  /** @param {readonly string[]} columns */
  forceNotNull(columns) {
    return this.#next(native.copyForceNotNull(arg(this), columns));
  }

  forceNotNullAll() {
    return this.#next(native.copyForceNotNullAll(arg(this)));
  }

  /** @param {readonly string[]} columns */
  forceNull(columns) {
    return this.#next(native.copyForceNull(arg(this), columns));
  }

  forceNullAll() {
    return this.#next(native.copyForceNullAll(arg(this)));
  }

  /** @param {readonly string[]} columns */
  forceQuote(columns) {
    return this.#next(native.copyForceQuote(arg(this), columns));
  }

  forceQuoteAll() {
    return this.#next(native.copyForceQuoteAll(arg(this)));
  }
}

/** A `COPY … FROM STDIN`, run by `copyIn`. Every method returns a new statement. */
export class CopyFrom extends CopyStatement {
  /**
   * @param {unknown} handle
   * @param {CopyFormat} format
   * @param {symbol} [token]
   */
  constructor(handle, format, token) {
    trusted(token, "CopyFrom", "copyFrom(table, format)");
    super(handle, format);
  }
}

/** A `COPY … TO STDOUT`, run by `copyOut`. Every method returns a new statement. */
export class CopyTo extends CopyStatement {
  /**
   * @param {unknown} handle
   * @param {CopyFormat} format
   * @param {symbol} [token]
   */
  constructor(handle, format, token) {
    trusted(token, "CopyTo", "copyTo(source, format)");
    super(handle, format);
  }
}

/**
 * `COPY table FROM STDIN` in `format`: a table named by a string or a
 * `Table` without an alias.
 *
 * @param {TableRef} table
 * @param {CopyFormat} format
 */
export function copyFrom(table, format) {
  return new CopyFrom(native.copyFromNew(arg(table), format), format, TRUSTED);
}

/**
 * `COPY … TO STDOUT` in `format`, reading a table, or a `Select`, or an
 * INSERT, UPDATE, DELETE or MERGE with a RETURNING list.
 *
 * @param {TableRef | import("./select.js").Select | import("./writes.js").Insert | import("./writes.js").Update | import("./writes.js").Delete | import("./merge.js").Merge} source
 * @param {CopyFormat} format
 */
export function copyTo(source, format) {
  return new CopyTo(native.copyToNew(arg(source), format), format, TRUSTED);
}

/**
 * @typedef {Uint8Array | Iterable<Uint8Array> | AsyncIterable<Uint8Array>} CopySource
 */

/**
 * What `copyIn` pulls its chunks from, checked before anything is sent: a
 * `Uint8Array`, or an iterable or async iterable of them — an array, a
 * generator, a web `ReadableStream`, a Node.js stream of buffers. A string is
 * iterable, but as characters, and is refused.
 *
 * @param {unknown} source
 * @returns {() => Iterator<unknown> | AsyncIterator<unknown>}
 */
export function copySource(source) {
  if (source instanceof Uint8Array) return () => [source][Symbol.iterator]();
  if (typeof source === "object" && source !== null) {
    const iterable = /** @type {{ [Symbol.asyncIterator]?: unknown, [Symbol.iterator]?: unknown }} */ (source);
    if (typeof iterable[Symbol.asyncIterator] === "function") {
      return () => /** @type {AsyncIterable<unknown>} */ (source)[Symbol.asyncIterator]();
    }
    if (typeof iterable[Symbol.iterator] === "function") {
      return () => /** @type {Iterable<unknown>} */ (source)[Symbol.iterator]();
    }
  }
  throw new TypeError(
    "a COPY's source is a Uint8Array, or an iterable or async iterable of them; a string is refused",
  );
}

/** The COPY opener a pool, a connection and a transaction each implement, keyed privately. */
export const COPY = Symbol("copy");

/**
 * What a pool, a connection and a transaction run COPY through. Each opens a
 * COPY in the addon with its `[COPY]` method, which resolves once the server
 * accepts the statement and holds what it runs on — a pool's connection of
 * its own, or the connection or transaction itself, which refuses other work
 * — until the COPY ends.
 */
export class CopyRunner {
  /**
   * @param {"in" | "out"} _direction
   * @param {unknown} _copy
   * @param {unknown} _token
   * @param {boolean} _rows
   * @returns {Promise<unknown>}
   */
  [COPY](_direction, _copy, _token, _rows) {
    return Promise.reject(new TypeError("COPY runs on a Pool, a Connection or a Transaction"));
  }

  /**
   * Release what an operation that finished after its abort holds: nothing
   * here, a transaction ending itself.
   *
   * @returns {Promise<void>}
   */
  async [LATE]() {}

  /**
   * Run a `COPY … FROM STDIN`, writing the bytes `source` yields — a
   * `Uint8Array`, or an iterable or async iterable of them — and resolve with
   * the rows it wrote. The source is checked before anything is sent, and
   * pulled only once the server has accepted the statement, and then only as
   * the COPY takes its data.
   *
   * @param {unknown} statement
   * @param {unknown} source
   * @param {unknown} [options]
   * @returns {Promise<number>}
   */
  async copyIn(statement, source, options = {}) {
    if (!(statement instanceof CopyFrom)) {
      throw new TypeError("copyIn runs a COPY FROM STDIN, from copyFrom(table, format)");
    }
    const { signal } = queryOptions(options);
    const open = copySource(source);
    return await abortable(signal, async (token) => {
      const handle = await this[COPY]("in", arg(statement), token, false);
      return await pump(handle, open, signal);
    }, () => this[LATE]());
  }

  /**
   * Run a `COPY … TO STDOUT` and resolve, once the server accepts it, with
   * its data as a stream of chunks, which holds what the COPY runs on until
   * it ends or closes.
   *
   * @param {unknown} statement
   * @param {unknown} [options]
   * @returns {Promise<CopyOutStream>}
   */
  async copyOut(statement, options = {}) {
    if (!(statement instanceof CopyTo)) {
      throw new TypeError("copyOut runs a COPY TO STDOUT, from copyTo(source, format)");
    }
    const { signal } = queryOptions(options);
    const handle = await abortable(
      signal,
      (token) => this[COPY]("out", arg(statement), token, false),
      (opened) => closeCopyOut(this, opened),
    );
    return new CopyOutStream(handle, this, signal, HANDLE);
  }

  /**
   * Run a binary `COPY … FROM STDIN` whose rows are arrays of values, one for
   * each of the columns the server describes for it, and resolve with the
   * rows it wrote. Each value is read as a parameter is, and written against
   * its column's type as a parameter is bound. A row of another width, or a
   * value its column's type does not take, is a `ConstructionError` that
   * abandons the COPY, as a source that throws does.
   *
   * @param {unknown} statement
   * @param {unknown} rows
   * @param {unknown} [options]
   * @returns {Promise<number>}
   */
  async copyInRows(statement, rows, options = {}) {
    if (!(statement instanceof CopyFrom) || statement.format !== "binary") {
      throw new TypeError('copyInRows runs a binary COPY FROM STDIN, from copyFrom(table, "binary")');
    }
    const { signal } = queryOptions(options);
    const open = rowSource(rows);
    return await abortable(signal, async (token) => {
      const handle = await this[COPY]("in", arg(statement), token, true);
      return await pumpRows(handle, open, signal);
    }, () => this[LATE]());
  }

  /**
   * Run a binary `COPY … TO STDOUT` and resolve, once the server accepts it,
   * with its rows as a stream of arrays of values, each field decoded by the
   * type the server describes for its column, as a result column of that
   * type is.
   *
   * @param {unknown} statement
   * @param {unknown} [options]
   * @returns {Promise<CopyRowStream>}
   */
  async copyOutRows(statement, options = {}) {
    if (!(statement instanceof CopyTo) || statement.format !== "binary") {
      throw new TypeError('copyOutRows runs a binary COPY TO STDOUT, from copyTo(source, "binary")');
    }
    const { signal } = queryOptions(options);
    const handle = await abortable(
      signal,
      (token) => this[COPY]("out", arg(statement), token, true),
      (opened) => closeCopyOut(this, opened),
    );
    return new CopyRowStream(handle, this, signal, HANDLE);
  }
}

/**
 * Close a COPY TO that opened after its abort, discarding the data it holds,
 * and release what `runner` ran it on.
 *
 * @param {CopyRunner} runner
 * @param {unknown} handle
 */
async function closeCopyOut(runner, handle) {
  await native.copyOutClose(handle);
  await runner[LATE]();
}

/**
 * A COPY TO's data as an async iterator of `Uint8Array` chunks, one per pull,
 * so the server is held back while nothing asks. The chunks are pieces of
 * one byte stream in the statement's format: a pull gives the data that has
 * arrived, and no chunk is promised to end at a row. The connection it holds
 * is released at the end, which frees it, or by `close()`, `return()` — a
 * `for await` loop leaving early — or a failed pull, which discard it if
 * data remained.
 *
 * @implements {AsyncIterableIterator<Uint8Array>}
 */
export class CopyOutStream {
  /** @type {unknown} */
  #native;
  /**
   * What the stream runs on, kept reachable while the stream is: a
   * transaction collected under its COPY would end it.
   *
   * @type {unknown}
   */
  #owner;
  /** @type {AbortSignal | undefined} */
  #signal;
  #done = false;
  #pulling = false;

  /**
   * @param {unknown} handle
   * @param {unknown} owner
   * @param {AbortSignal | undefined} signal
   * @param {symbol} trusted
   */
  constructor(handle, owner, signal, trusted) {
    if (trusted !== HANDLE) throw new TypeError("a CopyOutStream comes from copyOut()");
    this.#native = handle;
    this.#owner = owner;
    this.#signal = signal;
  }

  /** @returns {boolean} */
  get closed() {
    return this.#done || native.copyOutClosed(this.#native);
  }

  /** @returns {Promise<IteratorResult<Uint8Array, undefined>>} */
  async next() {
    if (this.#done) return { done: true, value: undefined };
    if (this.#pulling) throw new LifecycleError("the COPY stream is busy: one chunk is pulled at a time");
    this.#pulling = true;
    try {
      const chunk = await abortable(this.#signal, (token) => native.copyOutNext(this.#native, token));
      if (chunk === null) {
        this.#done = true;
        this.#owner = undefined;
        return { done: true, value: undefined };
      }
      return { done: false, value: /** @type {Uint8Array} */ (chunk) };
    } catch (error) {
      await this.#finish();
      throw error;
    } finally {
      this.#pulling = false;
    }
  }

  /**
   * End the stream, as a `for await` loop does when it leaves early.
   *
   * @returns {Promise<IteratorResult<Uint8Array, undefined>>}
   */
  async return() {
    await this.#finish();
    return { done: true, value: undefined };
  }

  /**
   * End the stream and release its connection: discarded if data remained.
   *
   * @returns {Promise<void>}
   */
  async close() {
    await this.#finish();
  }

  async #finish() {
    if (this.#done) return;
    this.#done = true;
    try {
      await native.copyOutClose(this.#native);
    } finally {
      this.#owner = undefined;
    }
  }

  [Symbol.asyncIterator]() {
    return this;
  }

  [Symbol.asyncDispose]() {
    return this.close();
  }
}
