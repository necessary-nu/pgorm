// @ts-check

// Row streams over a pool's or a connection's connection.
// [spec:pgorm:req:bindings.streams+2]

import { LifecycleError } from "./errors.js";
import { abortable, native, queryOptions, row, TRUSTED } from "./operations.js";

/**
 * How a stream makes each item from a row's values, given the result's
 * column names and kinds: a row object, unless a model decodes it.
 *
 * @typedef {(names: string[], values: unknown[], kinds: string[]) => unknown} Decode
 */

/** @type {(stream: RowStream, decode: Decode) => RowStream} */
let decoding;

/**
 * `stream`, its items made by `decode` rather than as row objects: how a
 * model streams its records. Only the module calls this, before the first
 * pull.
 * [spec:pgorm:req:bindings.model-records+2]
 *
 * @param {RowStream} stream
 * @param {Decode} decode
 */
export function decodedWith(stream, decode) {
  return decoding(stream, decode);
}

/**
 * A statement's rows as an async iterator, one row per pull, so the server is
 * held back while nothing asks. The addon hands over the rows it has
 * received in batches, which the stream yields from memory. Its connection is
 * released at the last row, by `close()`, or when a `for await` loop leaves
 * early.
 *
 * @implements {AsyncIterableIterator<Record<string, unknown>>}
 */
export class RowStream {
  /** @type {(token: unknown) => Promise<[unknown, (() => Promise<void>) | undefined]>} */
  #open;
  /** @type {unknown} */
  #native = null;
  /** @type {(() => Promise<void>) | undefined} */
  #release;
  /** @type {string[]} */
  #names = [];
  /** @type {string[]} */
  #kinds = [];
  /**
   * The rows of the addon's last batch, those from `#at` on not yet yielded.
   *
   * @type {unknown[]}
   */
  #batch = [];
  #at = 0;
  /** @type {Decode} */
  #decode = row;
  #tagged;
  /** @type {AbortSignal | undefined} */
  #signal;
  #done = false;
  #pulling = false;

  /**
   * @param {(token: unknown) => Promise<[unknown, (() => Promise<void>) | undefined]>} open
   * @param {unknown} options
   * @param {symbol} trusted
   */
  constructor(open, options, trusted) {
    if (trusted !== TRUSTED) throw new TypeError("a RowStream comes from stream()");
    const { tagged, signal } = queryOptions(options);
    this.#open = open;
    this.#tagged = tagged;
    this.#signal = signal;
  }

  static {
    decoding = (stream, decode) => {
      if (stream.#native !== null || stream.#done) throw new TypeError("the stream has been pulled");
      stream.#decode = decode;
      return stream;
    };
  }

  /** @returns {boolean} */
  get closed() {
    return this.#done;
  }

  /** @returns {Promise<IteratorResult<Record<string, unknown>, undefined>>} */
  async next() {
    if (this.#done) return { done: true, value: undefined };
    if (this.#pulling) throw new LifecycleError("the stream is busy: one row is pulled at a time");
    this.#pulling = true;
    try {
      if (this.#at < this.#batch.length && native.streamPullable(this.#native)) {
        const signal = this.#signal;
        if (signal !== undefined) {
          signal.throwIfAborted();
          // A pull's promise settles a turn later, as one the addon answers
          // does, so an abort right after the call still rejects it.
          await undefined;
          signal.throwIfAborted();
        }
        return this.#item(this.#batch[this.#at++]);
      }
      this.#batch = [];
      return await abortable(this.#signal, async (token) => {
        if (this.#native === null) [this.#native, this.#release] = await this.#open(token);
        const batch = await native.streamNext(this.#native, this.#tagged, token);
        if (batch === null) {
          await this.#finish(false);
          return { done: true, value: undefined };
        }
        const [rows, names, kinds] = batch;
        if (names) [this.#names, this.#kinds] = [names, kinds];
        [this.#batch, this.#at] = [rows, 1];
        return this.#item(rows[0]);
      });
    } catch (error) {
      await this.#finish();
      throw error;
    } finally {
      this.#pulling = false;
    }
  }

  /**
   * @param {unknown} values
   * @returns {IteratorResult<Record<string, unknown>, undefined>}
   */
  #item(values) {
    return {
      done: false,
      value: /** @type {Record<string, unknown>} */ (this.#decode(this.#names, /** @type {unknown[]} */ (values), this.#kinds)),
    };
  }

  /**
   * End the stream, as a `for await` loop does when it leaves early.
   *
   * @returns {Promise<IteratorResult<Record<string, unknown>, undefined>>}
   */
  async return() {
    await this.#finish();
    return { done: true, value: undefined };
  }

  /**
   * End the stream and release its connection: discarded if rows remained.
   *
   * @returns {Promise<void>}
   */
  async close() {
    await this.#finish();
  }

  /**
   * @param {boolean} [close] whether the addon's stream is to be closed:
   * not once a pull found its end, which released what it held.
   */
  async #finish(close = true) {
    if (this.#done) return;
    this.#done = true;
    this.#batch = [];
    try {
      if (close && this.#native !== null) await native.streamClose(this.#native);
    } finally {
      await this.#release?.();
    }
  }

  [Symbol.asyncIterator]() {
    return this;
  }

  [Symbol.asyncDispose]() {
    return this.close();
  }
}
