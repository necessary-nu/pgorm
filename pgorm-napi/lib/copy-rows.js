// @ts-check

// Binary COPY as rows of values: the rows `copyInRows` writes, encoded by the
// addon a batch at a time against the column types the server describes, and
// the `CopyRowStream` `copyOutRows` reads them back through.
// [spec:pgorm:req:bindings.copy-rows+2]

import { close, describe, step, watching, write } from "./copy-feed.js";
import { LifecycleError } from "./errors.js";
import { decoder } from "./model-decode.js";
import { shapeOf } from "./model-shape.js";
import { abortable, native, TRUSTED as HANDLE } from "./operations.js";

/**
 * How many rows cross into the addon to be encoded at once: few enough that a
 * slow source's rows are not held back long, many enough that a fast one
 * costs few calls.
 */
const BATCH = 256;

/**
 * What `copyInRows` pulls its rows from, checked before anything is sent: an
 * iterable or async iterable of arrays. A `Uint8Array` and a string are
 * iterable, but are no rows, and are refused.
 *
 * @param {unknown} rows
 * @returns {() => Iterator<unknown> | AsyncIterator<unknown>}
 */
export function rowSource(rows) {
  if (typeof rows === "object" && rows !== null && !(rows instanceof Uint8Array)) {
    const iterable = /** @type {{ [Symbol.asyncIterator]?: unknown, [Symbol.iterator]?: unknown }} */ (rows);
    if (typeof iterable[Symbol.asyncIterator] === "function") {
      return () => /** @type {AsyncIterable<unknown>} */ (rows)[Symbol.asyncIterator]();
    }
    if (typeof iterable[Symbol.iterator] === "function") {
      return () => /** @type {Iterable<unknown>} */ (rows)[Symbol.iterator]();
    }
  }
  throw new TypeError("copyInRows reads an iterable or async iterable of rows, each an array of values");
}

/**
 * Encode `batch` in the addon, which reads each value as a parameter is read
 * and writes it against its column's type, and queue it for the COPY.
 *
 * @param {unknown} handle
 * @param {unknown[][]} batch
 */
async function writeRows(handle, batch) {
  if (batch.length > 0) await write(handle, native.copyRowsEncode(handle, batch));
}

/**
 * Feed a COPY FROM of rows as `pump` feeds one of bytes: each row an array of
 * its columns' values, gathered into batches the addon encodes, the format's
 * header before the first and its trailer at the finish. A row that is no
 * array, of another width, or holding a value its column's type does not
 * take abandons the COPY, as a source that throws does.
 *
 * @param {unknown} handle
 * @param {() => Iterator<unknown> | AsyncIterator<unknown>} open
 * @param {AbortSignal | undefined} signal
 * @returns {Promise<number>}
 */
export async function pumpRows(handle, open, signal) {
  /** @type {Iterator<unknown> | AsyncIterator<unknown> | undefined} */
  let iterator;
  let pulling = false;
  const watch = watching(native.copyInEnded(handle));
  try {
    iterator = open();
    /** @type {unknown[][]} */
    let batch = [];
    for (;;) {
      pulling = true;
      const next = await step(iterator, signal, watch);
      pulling = false;
      if (next.done) break;
      const row = next.value;
      if (!Array.isArray(row)) {
        throw new TypeError(`a COPY row is an array of its columns' values, not ${describe(row)}`);
      }
      batch.push(row);
      if (batch.length >= BATCH) {
        await writeRows(handle, batch);
        batch = [];
      }
    }
    await writeRows(handle, batch);
  } catch (error) {
    await native.copyInAbort(handle);
    if (iterator !== undefined && !pulling) await close(iterator);
    else if (iterator !== undefined && !watch.sourceFailed) void close(iterator);
    throw error;
  }
  return await native.copyInFinish(handle);
}

/**
 * Each row `rows` yields, as a record of the model `shape` is: each field found by its
 * column among the COPY's, which may hold columns no field reads, and held to
 * its kind as a model's terminals hold it. A row that does not decode closes
 * `rows`, as leaving the iteration early does.
 *
 * @param {CopyRowStream} rows
 * @param {import("./model-shape.js").Shape} shape
 * @param {unknown} handle
 */
async function* recordsOf(rows, shape, handle) {
  const decode = decoder(
    [{ fields: shape.fields, names: shape.fields.map((field) => field.sql), optional: false }],
    { exhaustive: false, castsText: false },
  );
  /** @type {string[] | undefined} */
  let kinds;
  for await (const row of rows) {
    kinds ??= native.copyRowsKinds(handle);
    yield decode(rows.columns, row, /** @type {string[]} */ (kinds));
  }
}

/**
 * A binary COPY TO's rows as an async iterator of arrays, one row per pull,
 * each value decoded by its column's type as a result column of that type is.
 * Rows are fetched from the addon as they arrive, a batch at a time, so the
 * server is held back while nothing asks. `columns` names the columns each
 * row holds, and `models(model)` reads the rows as a model's records
 * instead. The connection it holds is released at the end, or by `close()`,
 * `return()` or a failed pull, which discard it if data remained.
 *
 * @implements {AsyncIterableIterator<unknown[]>}
 */
export class CopyRowStream {
  /** @type {unknown} */
  #native;
  /** @type {unknown} */
  #owner;
  /** @type {AbortSignal | undefined} */
  #signal;
  /** @type {unknown[][]} */
  #rows = [];
  #at = 0;
  #done = false;
  #pulling = false;
  /** @type {readonly string[]} */
  #columns;

  /**
   * @param {unknown} handle
   * @param {unknown} owner
   * @param {AbortSignal | undefined} signal
   * @param {symbol} trusted
   */
  constructor(handle, owner, signal, trusted) {
    if (trusted !== HANDLE) throw new TypeError("a CopyRowStream comes from copyOutRows()");
    this.#native = handle;
    this.#owner = owner;
    this.#signal = signal;
    this.#columns = Object.freeze(/** @type {string[]} */ (native.copyRowsColumns(handle)));
  }

  /** The names of the columns each row holds, in order. */
  get columns() {
    return this.#columns;
  }

  /** @returns {boolean} */
  get closed() {
    return this.#done || native.copyOutClosed(this.#native);
  }

  /**
   * The rows still to come as records of `model`, pulled from this stream.
   *
   * @param {unknown} model
   * @returns {AsyncGenerator<Record<string, unknown>, void, undefined>}
   */
  models(model) {
    return recordsOf(this, shapeOf(model), this.#native);
  }

  /** @returns {Promise<IteratorResult<unknown[], undefined>>} */
  async next() {
    if (this.#at < this.#rows.length) {
      const row = /** @type {unknown[]} */ (this.#rows[this.#at]);
      this.#at += 1;
      return { done: false, value: row };
    }
    if (this.#done) return { done: true, value: undefined };
    if (this.#pulling) throw new LifecycleError("the COPY stream is busy: one row is pulled at a time");
    this.#pulling = true;
    try {
      const rows = await abortable(this.#signal, (token) => native.copyRowsNext(this.#native, token));
      if (rows === null) {
        this.#done = true;
        this.#owner = undefined;
        return { done: true, value: undefined };
      }
      this.#rows = /** @type {unknown[][]} */ (rows);
      this.#at = 1;
      return { done: false, value: /** @type {unknown[]} */ (this.#rows[0]) };
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
   * @returns {Promise<IteratorResult<unknown[], undefined>>}
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
    this.#rows = [];
    this.#at = 0;
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
